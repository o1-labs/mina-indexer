//! V2 transaction-hash computation.
//!
//! Mirrors mina's `Signed_command.Stable.V2.t` as a set of tagless leaf types,
//! bin_prot-serializes them, and Blake2b's the result — exactly what mina's
//! `Transaction_hash.hash_signed_command` does. zkApp commands use a different
//! (`Zkapp_command`) hasher that isn't ported yet; they fall back to the prior
//! JSON digest so their behavior is unchanged.

use super::TxnHash;
use crate::{
    mina_blocks::v2::staged_ledger_diff::{SignedCommandPayloadBody, UserCommandData},
    proof_systems::signer::pubkey::CompressedPubKey,
    protocol::{
        bin_prot,
        serialization_types::{
            signatures::CompressedCurvePoint, staged_ledger_diff::SignedCommandMemo,
            version_bytes::V2_TXN_HASH,
        },
    },
};
use anyhow::Result;
use blake2::{digest::VariableOutput, Blake2bVar};
use std::io::Write;

// Tagless mirror of mina's `Signed_command.Stable.V2.t`, which is what the v2
// transaction hash serializes (the bare `Stable.V2.bin_io` has NO version tags;
// `with_top_version_tag` only affects the separate `.With_top_version_tag`
// submodule). We assemble the leaf types bare and Blake2b the result, with the
// signature dummied to `(Field.one, Scalar.one)` exactly as mina's
// `Transaction_hash.hash_signed_command` does.

#[derive(serde::Serialize)]
struct V2HashSignedCommand {
    payload: V2HashPayload,
    // `Public_key.Stable.V1` = `Non_zero_curve_point.Stable.V1`, whose bin_io is
    // the COMPRESSED `{x, is_odd}` form (33 bytes).
    signer: CompressedCurvePoint,
    signature: ([u8; 32], [u8; 32]),
}

#[derive(serde::Serialize)]
struct V2HashPayload {
    common: V2HashCommon,
    body: V2HashBody,
}

#[derive(serde::Serialize)]
struct V2HashCommon {
    fee: u64,
    fee_payer_pk: CompressedCurvePoint,
    nonce: i32,
    valid_until: V2GlobalSlot,
    memo: SignedCommandMemo,
}

// `Global_slot_since_genesis.Stable.V1 = Since_genesis of uint32` — a single-
// constructor variant, so its bin_io is `[tag 0][int32 value]`. (Account_nonce
// is a plain uint32, no wrapper — that's the asymmetry.)
#[derive(serde::Serialize)]
enum V2GlobalSlot {
    SinceGenesis(i32),
}

#[derive(serde::Serialize)]
enum V2HashBody {
    Payment {
        receiver_pk: CompressedCurvePoint,
        amount: u64,
    },
    StakeDelegation(V2HashSetDelegate),
}

#[derive(serde::Serialize)]
enum V2HashSetDelegate {
    SetDelegate { new_delegate: CompressedCurvePoint },
}

fn v2_compressed(pk: &str) -> Result<CompressedCurvePoint> {
    Ok((&CompressedPubKey::from_address(pk)?).into())
}

/// Builds the tagless `Signed_command.Stable.V2` structure for hashing, with
/// the dummy signature. Blake2b of its bin_io is mina's canonical v2 txn hash.
fn build_v2_hashable(v2: &UserCommandData) -> Result<V2HashSignedCommand> {
    let UserCommandData::SignedCommandData(data) = v2 else {
        anyhow::bail!("not a signed command");
    };
    let common = &data.payload.common;

    // base58check memo -> raw 34-byte memo (strip version byte + 4-byte checksum)
    let decoded = bs58::decode(&common.memo).into_vec()?;
    if decoded.len() < 5 {
        anyhow::bail!("v2 memo base58 decoded to {} bytes (< 5)", decoded.len());
    }
    let memo = SignedCommandMemo(decoded[1..decoded.len() - 4].to_vec());

    let body = match &data.payload.body.1 {
        SignedCommandPayloadBody::Payment(payment) => V2HashBody::Payment {
            receiver_pk: v2_compressed(&payment.receiver_pk.0)?,
            amount: payment.amount.0,
        },
        SignedCommandPayloadBody::StakeDelegation((_, delegation)) => {
            V2HashBody::StakeDelegation(V2HashSetDelegate::SetDelegate {
                new_delegate: v2_compressed(&delegation.new_delegate.0)?,
            })
        }
    };

    // Signature.dummy = (Field.one, Scalar.one); canonical field bytes (LE) of 1.
    let one = {
        let mut b = [0u8; 32];
        b[0] = 1;
        b
    };
    Ok(V2HashSignedCommand {
        payload: V2HashPayload {
            common: V2HashCommon {
                fee: common.fee.0,
                fee_payer_pk: v2_compressed(&common.fee_payer_pk.0)?,
                nonce: common.nonce.0 as i32,
                valid_until: V2GlobalSlot::SinceGenesis(common.valid_until.0 as i32),
                memo,
            },
            body,
        },
        signer: v2_compressed(&data.signer.0)?,
        signature: (one, one),
    })
}

pub fn hash_command_v2(v2: &UserCommandData) -> Result<TxnHash> {
    // zkApp commands use a different (Zkapp_command) hasher — not yet ported.
    if matches!(v2, UserCommandData::ZkappCommandData(_)) {
        return hash_command_v2_zkapp_fallback(v2);
    }
    let sc = build_v2_hashable(v2)?;

    let mut bytes = Vec::new();
    bin_prot::to_writer(&mut bytes, &sc)?;

    let mut hasher = Blake2bVar::new(32)?;
    hasher.write_all(&bytes)?;
    let mut hash = hasher.finalize_boxed().to_vec();
    hash.insert(0, hash.len() as u8);

    Ok(TxnHash::V2(
        bs58::encode(hash)
            .with_check_version(V2_TXN_HASH)
            .into_string(),
    ))
}

/// Placeholder for zkApp-command hashing (mina's `Zkapp_command` hasher). Kept
/// as the previous JSON digest so behavior for zkApps is unchanged until
/// ported.
fn hash_command_v2_zkapp_fallback(v2: &UserCommandData) -> Result<TxnHash> {
    let bytes = serde_json::to_vec(v2)?;
    let mut hasher = Blake2bVar::new(32)?;
    hasher.write_all(&bytes[..])?;
    let mut hash = hasher.finalize_boxed().to_vec();
    hash.insert(0, hash.len() as u8);
    Ok(TxnHash::V2(
        bs58::encode(hash)
            .with_check_version(V2_TXN_HASH)
            .into_string(),
    ))
}
