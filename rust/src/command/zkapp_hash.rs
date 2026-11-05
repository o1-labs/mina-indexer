//! zkApp command transaction hash.
//!
//! Mirrors mina's `Transaction_hash.hash_zkapp_command`
//! (`src/lib/transaction/transaction_hash.ml`). This module exists for one
//! reason: the precomputed block JSON does not contain the hash of a zkApp
//! command, so the indexer must calculate it, and the result must be equal to
//! the hash that the archive node and explorers show.
//!
//! # The hash
//!
//! 1. Replace every signature and proof with a dummy value.
//! 2. Serialize `Zkapp_command.Stable.Latest` with bin_prot.
//! 3. Calculate Blake2b-256 of those bytes.
//! 4. Encode the digest as base58check.
//!
//! One wrong byte gives a completely different hash. There is no partial
//! success, so the tests compare against hashes from real blocks.
//!
//! # Why bin_prot
//!
//! bin_prot is the binary format of the OCaml `Stable` types. The protocol
//! defines the hash over those bytes, so no other format can give the same
//! hash. The JSON in a block is only a view of the same data.
//!
//! # Why dummy signatures and proofs
//!
//! Mina's reason, from the comment in `transaction_hash.ml`: this lets it
//! "reproduce the transaction hashes if signatures, proofs omitted in archive
//! db". The hash covers the *kind* of authorization, not its content. A proof
//! stays a proof, a signature stays a signature, and `None_given` stays
//! `None_given`.
//!
//! `DUMMY_PROOF` is stored as ready-made bytes. It is
//! `Proof.transaction_dummy`, and the indexer has no prover to make it.
//!
//! # Why this module does not use `bin_prot::to_writer`
//!
//! There are two levels of bin_prot in this crate:
//!
//! | Level | Location | Needs |
//! |---|---|---|
//! | Primitives | `protocol::bin_prot::WriteBinProtExt` | any `io::Write` |
//! | Serde serializer | `protocol::bin_prot::to_writer` | Rust types with the shape of the OCaml types |
//!
//! The types in `mina_blocks::v2` have the shape of the JSON, not the shape of
//! the wire format. `to_writer` needs a complete set of mirror types for zkApp
//! commands, so `Writer` walks the JSON types by hand.
//!
//! **Do not write bin_prot primitives here.** `Writer` uses `WriteBinProtExt`
//! for those, and adds only what the primitives cannot know: the Mina rule
//! that selects an encoding for each field.
//!
//! # The format has no field names
//!
//! bin_prot writes no field names and no field count: a record is its fields,
//! in the order of the OCaml definition. The order of the calls in `Writer`
//! *is* the format. If you move one call, the hash changes.
//!
//! # Encoding rules
//!
//! `cargo nextest` does not run doctests, so these examples are plain text. The
//! test `doc_examples` checks the slot, unsigned, string and `Vector.t` bytes:
//! change the two places together. The real blocks in `mesa_block_hashes` and
//! `berkeley_block_hashes` cover the variant and call forest rules.
//!
//! **Variants** are a one-byte constructor index, then the value. The index is
//! the position in the OCaml definition, so read that definition. The value
//! with no data is not always first:
//!
//! ```text
//! Option:      None = 00             Some x    = 01 x
//! Set_or_keep: Set x  = 00 x         Keep      = 01
//! Or_ignore:   Check x = 00 x        Ignore    = 01
//! ```
//!
//! **Single-constructor variants** still write the index. This includes
//! `Global_slot_since_genesis` and `Global_slot_span`. The JSON shows only a
//! number:
//!
//! ```text
//! slot 5:      00 05
//! ```
//!
//! **Unsigned values** are `Unsigned_extended.UInt32` and `UInt64`, which
//! bin_prot stores as its signed `int32` and `int64`. Wrap the value to signed
//! first, because the shortest encoding depends on the signed value:
//!
//! ```text
//! uint32 40,000:          fd 40 9c 00 00
//! uint32 3,000,000,000:   fd 00 5e d0 b2     (without the wrap: fc + 8 bytes)
//! uint64 u64::MAX:        ff ff              (it wraps to -1)
//! ```
//!
//! **Lists and strings** have a nat0 length first, then the content:
//!
//! ```text
//! list [a, b]:   02 a b
//! string "hi":   02 68 69
//! ```
//!
//! **`Vector.t`** has no length. Mina writes it as nested pairs that end in
//! unit, `(a, (b, ()))` (`src/lib/crypto/plonkish_prelude/vector.ml`). A pair
//! writes nothing itself and unit is a zero byte, so:
//!
//! ```text
//! Vector [a, b]: a b 00
//! ```
//!
//! This code writes the elements that the block gives, so the same path covers
//! the 8 field Berkeley format and the 32 field mesa format.
//!
//! **Digests are unit.** The hash uses `Zkapp_command.Call_forest` with the
//! digests forgotten, so each `account_update_digest` and `stack_hash` field
//! is unit, which is a zero byte:
//!
//! ```text
//! forest:  nat0 count, then each (tree, 00)
//! tree:    account_update, 00, forest of calls
//! ```
//!
//! # Field elements
//!
//! A field is 32 little-endian bytes on the wire. The JSON has three forms,
//! which `Writer::field` handles: decimal, `0x` big-endian hex, and base58check
//! with the field as the last 32 payload bytes.
//!
//! # When the protocol changes
//!
//! A new `Stable` version can add or move fields. Compare the OCaml types with
//! `Writer`, then add a block from the new version to
//! `rust/tests/data/zkapp_hash`. A hash that does not match means that a byte
//! is wrong: compare the output of `zkapp_command_bytes` with
//! `Binable.to_string` from the Mina code.

use super::TxnHash;
use crate::{
    mina_blocks::v2::{
        protocol_state::SupplyAdjustmentSign,
        staged_ledger_diff::{
            AccountPreconditions, AccountUpdate, AccountUpdateBody, Authorization, Elt,
            LedgerPreconditions, MayUseToken, NetworkPreconditions, NumericBoundsU32,
            NumericBoundsU64, Precondition, Preconditions, ProofOrSignature,
            StakingEpochDataPreconditions, Update, UpdateKind, UpdatePermissions, UpdateTiming,
            UpdateVerificationKey, ZkappCommandData,
        },
        Permission, PermissionKind, Permissions,
    },
    protocol::{bin_prot::WriteBinProtExt, serialization_types::version_bytes::V2_TXN_HASH},
};
use anyhow::{anyhow, bail, Context, Result};
use blake2::{digest::VariableOutput, Blake2bVar};
use std::io::Write;

/// bin_prot of `Proof.transaction_dummy`, from mina-rust
/// (`crates/p2p-messages/src/v2/dummy`, Apache-2.0)
const DUMMY_PROOF: &[u8] = include_bytes!("dummy_transaction_proof.bin");

/// `Signature.dummy` = `(Field.one, Scalar.one)`
const DUMMY_SIGNATURE: [u8; 64] = {
    let mut sig = [0; 64];
    sig[0] = 1;
    sig[32] = 1;
    sig
};

const FIELD_LEN: usize = 32;

const BASE58_CHECKSUM_LEN: usize = 4;

pub fn hash_zkapp_command(zkapp: &ZkappCommandData) -> Result<TxnHash> {
    let bytes = zkapp_command_bytes(zkapp)?;

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

fn zkapp_command_bytes(zkapp: &ZkappCommandData) -> Result<Vec<u8>> {
    let mut w = Writer { out: vec![] };

    // fee payer
    let fee_payer = &zkapp.fee_payer;
    w.public_key(&fee_payer.body.public_key.0)?;
    w.uint64(fee_payer.body.fee.0)?;
    match &fee_payer.body.valid_until {
        Some(slot) => {
            w.tag(1)?;
            w.slot(slot.0 as u32)?;
        }
        None => w.tag(0)?,
    }
    w.uint32(fee_payer.body.nonce.0)?;
    w.out.extend(DUMMY_SIGNATURE);

    // call forest
    w.out.bin_write_nat0(zkapp.account_updates.len() as u64)?;
    for update in &zkapp.account_updates {
        w.tree(&update.elt)?;
        w.out.bin_write_unit()?;
    }

    w.memo(&zkapp.memo)?;
    Ok(w.out)
}

/// Some producers write hashes as decimal field elements
fn decimal_to_le_bytes(value: &str) -> Result<[u8; FIELD_LEN]> {
    let mut bytes = [0u8; FIELD_LEN];
    for digit in value.bytes() {
        let mut carry = (digit - b'0') as u32;
        for byte in bytes.iter_mut() {
            let next = *byte as u32 * 10 + carry;
            *byte = next as u8;
            carry = next >> 8;
        }
        if carry != 0 {
            bail!("field {value} does not fit in {FIELD_LEN} bytes");
        }
    }
    Ok(bytes)
}

/// Writes `Zkapp_command.Stable.Latest` from the block JSON types. The order of
/// the calls is the wire format: see the module docs.
struct Writer {
    out: Vec<u8>,
}

impl Writer {
    ///////////////////
    // mina encoding //
    ///////////////////

    /// A constructor index. See "Variants" in the module docs.
    fn tag(&mut self, tag: u8) -> Result<()> {
        self.out.bin_write_variant_index(tag)?;
        Ok(())
    }

    /// `Unsigned_extended.UInt32`, which is bin_prot's `int32`
    fn uint32(&mut self, value: u32) -> Result<()> {
        self.out.bin_write_integer(value as i32)?;
        Ok(())
    }

    /// `Unsigned_extended.UInt64`, which is bin_prot's `int64`
    fn uint64(&mut self, value: u64) -> Result<()> {
        self.out.bin_write_integer(value as i64)?;
        Ok(())
    }

    /// `Global_slot_since_genesis.Stable.V1 = Since_genesis of uint32`
    fn slot(&mut self, value: u32) -> Result<()> {
        self.tag(0)?;
        self.uint32(value)
    }

    /// `Vector.t` has no length prefix: its elements, then a zero byte. See the
    /// module docs.
    fn vector_end(&mut self) -> Result<()> {
        self.out.bin_write_unit()?;
        Ok(())
    }

    fn string(&mut self, value: &[u8]) -> Result<()> {
        self.out.bin_write_nat0(value.len() as u64)?;
        self.out.extend(value);
        Ok(())
    }

    ///////////////////
    // mina leaves   //
    ///////////////////

    /// A field element: little-endian bytes. JSON has big-endian hex, or a
    /// base58check hash with the field as its last 32 payload bytes.
    fn field(&mut self, value: &str) -> Result<()> {
        if value.bytes().all(|byte| byte.is_ascii_digit()) {
            self.out.extend(decimal_to_le_bytes(value)?);
        } else if let Some(hex) = value.strip_prefix("0x") {
            let mut bytes = hex::decode(hex).map_err(|e| anyhow!("field {value}: {e}"))?;
            if bytes.len() != FIELD_LEN {
                bail!("field {value} is not {FIELD_LEN} bytes");
            }
            bytes.reverse();
            self.out.extend(bytes);
        } else {
            let bytes = bs58::decode(value)
                .into_vec()
                .with_context(|| format!("hash {value}"))?;
            let end = bytes
                .len()
                .checked_sub(BASE58_CHECKSUM_LEN)
                .filter(|end| *end >= FIELD_LEN)
                .ok_or_else(|| anyhow!("hash {value} is too short"))?;
            self.out.extend(&bytes[end - FIELD_LEN..end]);
        }
        Ok(())
    }

    /// `{x, is_odd}`
    fn public_key(&mut self, value: &str) -> Result<()> {
        let pk = crate::proof_systems::signer::pubkey::CompressedPubKey::from_address(value)?;
        let point: crate::protocol::serialization_types::signatures::CompressedCurvePoint =
            (&pk).into();
        self.out.extend(point.x);
        self.out.bin_write_bool(point.is_odd)?;
        Ok(())
    }

    fn memo(&mut self, value: &str) -> Result<()> {
        let decoded = bs58::decode(value).into_vec()?;
        if decoded.len() < 1 + BASE58_CHECKSUM_LEN {
            bail!("memo {value} is too short");
        }
        self.string(&decoded[1..decoded.len() - BASE58_CHECKSUM_LEN])?;
        Ok(())
    }

    ///////////////////
    // call forest   //
    ///////////////////

    /// `{account_update; account_update_digest = (); calls}`
    fn tree(&mut self, elt: &Elt) -> Result<()> {
        self.account_update(&elt.account_update)?;
        self.out.bin_write_unit()?;
        self.out.bin_write_nat0(elt.calls.len() as u64)?;
        for call in &elt.calls {
            self.tree(&call.elt)?;
            self.out.bin_write_unit()?;
        }
        Ok(())
    }

    fn account_update(&mut self, update: &AccountUpdate) -> Result<()> {
        self.body(&update.body)?;
        self.control(&update.authorization)?;
        Ok(())
    }

    /// `Control`: `Proof of _ | Signature of _ | None_given`, all dummied
    fn control(&mut self, auth: &Authorization) -> Result<()> {
        use Authorization::*;
        match auth {
            Proof_((ProofOrSignature::Proof, _)) | Signature_((ProofOrSignature::Proof, _)) => {
                self.tag(0)?;
                self.out.extend(DUMMY_PROOF);
            }
            Proof_((ProofOrSignature::Signature, _))
            | Signature_((ProofOrSignature::Signature, _)) => {
                self.tag(1)?;
                self.out.extend(DUMMY_SIGNATURE);
            }
            NoneGiven(_) | Either(_) | Proof(_) | Signature(_) => self.tag(2)?,
        }
        Ok(())
    }

    /// `Authorization_kind`: `Signature | Proof of field | None_given`
    fn authorization_kind(&mut self, kind: &Authorization) -> Result<()> {
        use Authorization::*;
        match kind {
            NoneGiven((name,)) | Either((name,)) | Proof((name,)) | Signature((name,)) => {
                match name.as_str() {
                    "Signature" => self.tag(0)?,
                    "None_given" => self.tag(2)?,
                    other => bail!("unexpected authorization kind {other}"),
                }
                Ok(())
            }
            Proof_((_, vk_hash)) | Signature_((_, vk_hash)) => {
                self.tag(1)?;
                self.field(vk_hash)
            }
        }
    }

    fn body(&mut self, body: &AccountUpdateBody) -> Result<()> {
        self.public_key(&body.public_key.0)?;
        self.field(&body.token_id.0)?;
        self.update(&body.update)?;

        self.uint64(body.balance_change.magnitude.0)?;
        self.out
            .bin_write_variant_index(match body.balance_change.sgn.0 {
                SupplyAdjustmentSign::Pos => 0,
                SupplyAdjustmentSign::Neg => 1,
            })?;

        self.out.bin_write_bool(body.increment_nonce)?;

        self.out.bin_write_nat0(body.events.len() as u64)?;
        for event in &body.events {
            self.fields(&event.0)?;
        }
        self.out.bin_write_nat0(body.actions.len() as u64)?;
        for action in &body.actions {
            self.fields(&action.0)?;
        }

        self.field(&body.call_data)?;
        self.preconditions(&body.preconditions)?;
        self.out.bin_write_bool(body.use_full_commitment)?;
        self.out
            .bin_write_bool(body.implicit_account_creation_fee)?;
        self.out
            .bin_write_variant_index(match body.may_use_token.0 {
                MayUseToken::No => 0,
                MayUseToken::ParentsOwnToken => 1,
                MayUseToken::InheritFromParent => 2,
            })?;
        self.authorization_kind(&body.authorization_kind)
    }

    fn fields(&mut self, fields: &[String]) -> Result<()> {
        self.out.bin_write_nat0(fields.len() as u64)?;
        fields.iter().try_for_each(|field| self.field(field))
    }

    ///////////////////
    // update        //
    ///////////////////

    /// `Set_or_keep`: `Set of _ | Keep`
    fn set_or_keep(
        &mut self,
        value: &UpdateKind,
        f: impl FnOnce(&mut Self, &str) -> Result<()>,
    ) -> Result<()> {
        match value {
            UpdateKind::Set((_, value)) => {
                self.tag(0)?;
                f(self, value)
            }
            UpdateKind::Keep(_) => {
                self.tag(1)?;
                Ok(())
            }
        }
    }

    fn update(&mut self, update: &Update) -> Result<()> {
        for state in &update.app_state {
            self.set_or_keep(state, Self::field)?;
        }
        self.vector_end()?;
        self.set_or_keep(&update.delegate, Self::public_key)?;

        match &update.verification_key {
            UpdateVerificationKey::Set((_, vk)) => {
                self.tag(0)?;
                let decoded = bs58::decode(&vk.data.0).with_check(None).into_vec()?;
                self.out.extend(&decoded[1..]);
            }
            UpdateVerificationKey::Keep(_) => self.tag(1)?,
        }

        match &update.permissions {
            UpdatePermissions::Set((_, permissions)) => {
                self.tag(0)?;
                self.permissions(permissions)?;
            }
            UpdatePermissions::Keep(_) => self.tag(1)?,
        }

        self.set_or_keep(&update.zkapp_uri, |w, uri| {
            w.string(uri.as_bytes())?;
            Ok(())
        })?;
        self.set_or_keep(&update.token_symbol, |w, symbol| {
            w.string(symbol.as_bytes())?;
            Ok(())
        })?;

        match &update.timing {
            UpdateTiming::Set((_, timing)) => {
                self.tag(0)?;
                self.uint64(timing.initial_minimum_balance.0)?;
                self.slot(timing.cliff_time.0)?;
                self.uint64(timing.cliff_amount.0)?;
                // `Global_slot_span` is a single-constructor variant too
                self.slot(timing.vesting_period.0)?;
                self.uint64(timing.vesting_increment.0)?;
            }
            UpdateTiming::Keep(_) => self.tag(1)?,
        }

        self.set_or_keep(&update.voting_for, Self::field)
    }

    fn permission(&mut self, permission: &Permission) -> Result<()> {
        self.tag(match permission.0 {
            PermissionKind::None => 0,
            PermissionKind::Either => 1,
            PermissionKind::Proof => 2,
            PermissionKind::Signature => 3,
            PermissionKind::Impossible => 4,
        })?;
        Ok(())
    }

    fn permissions(&mut self, p: &Permissions) -> Result<()> {
        self.permission(&p.edit_state)?;
        self.permission(&p.access)?;
        self.permission(&p.send)?;
        self.permission(&p.receive)?;
        self.permission(&p.set_delegate)?;
        self.permission(&p.set_permissions)?;
        self.permission(&p.set_verification_key.0)?;
        self.uint32(p.set_verification_key.1.parse()?)?;
        self.permission(&p.set_zkapp_uri)?;
        self.permission(&p.edit_action_state)?;
        self.permission(&p.set_token_symbol)?;
        self.permission(&p.increment_nonce)?;
        self.permission(&p.set_voting_for)?;
        self.permission(&p.set_timing)?;
        Ok(())
    }

    ///////////////////
    // preconditions //
    ///////////////////

    /// `Or_ignore`: `Check of _ | Ignore`
    fn or_ignore<T>(
        &mut self,
        value: &Precondition<T>,
        f: impl FnOnce(&mut Self, &T) -> Result<()>,
    ) -> Result<()> {
        match value {
            Precondition::Check((_, value)) => {
                self.tag(0)?;
                f(self, value)
            }
            Precondition::Ignore(_) => {
                self.tag(1)?;
                Ok(())
            }
        }
    }

    fn hash_check(&mut self, value: &Precondition<String>) -> Result<()> {
        self.or_ignore(value, |w, hash| w.field(hash))
    }

    fn length(&mut self, value: &Precondition<NumericBoundsU32>) -> Result<()> {
        self.or_ignore(value, |w, bounds| {
            w.uint32(bounds.lower.0)?;
            w.uint32(bounds.upper.0)?;
            Ok(())
        })
    }

    fn global_slot(&mut self, value: &Precondition<NumericBoundsU32>) -> Result<()> {
        self.or_ignore(value, |w, bounds| {
            w.slot(bounds.lower.0)?;
            w.slot(bounds.upper.0)?;
            Ok(())
        })
    }

    fn amount(&mut self, value: &Precondition<NumericBoundsU64>) -> Result<()> {
        self.or_ignore(value, |w, bounds| {
            w.uint64(bounds.lower.0)?;
            w.uint64(bounds.upper.0)?;
            Ok(())
        })
    }

    fn epoch_data(&mut self, data: &StakingEpochDataPreconditions) -> Result<()> {
        let LedgerPreconditions {
            hash,
            total_currency,
        } = &data.ledger;
        self.hash_check(hash)?;
        self.amount(total_currency)?;
        self.hash_check(&data.seed)?;
        self.hash_check(&data.start_checkpoint)?;
        self.hash_check(&data.lock_checkpoint)?;
        self.length(&data.epoch_length)
    }

    fn network(&mut self, network: &NetworkPreconditions) -> Result<()> {
        self.or_ignore(&network.snarked_ledger_hash, |w, hash| w.field(&hash.0))?;
        self.length(&network.blockchain_length)?;
        self.length(&network.min_window_density)?;
        self.amount(&network.total_currency)?;
        self.global_slot(&network.global_slot_since_genesis)?;
        self.epoch_data(&network.staking_epoch_data)?;
        self.epoch_data(&network.next_epoch_data)
    }

    fn account(&mut self, account: &AccountPreconditions) -> Result<()> {
        self.amount(&account.balance)?;
        self.length(&account.nonce)?;
        self.hash_check(&account.receipt_chain_hash)?;
        self.or_ignore(&account.delegate, |w, pk| w.public_key(&pk.0))?;

        for state in &account.state {
            self.or_ignore(state, |w, state| w.field(&state.0))?;
        }
        self.vector_end()?;

        self.hash_check(&account.action_state)?;
        self.or_ignore(&account.proved_state, |w, value| {
            w.out.bin_write_bool(*value)?;
            Ok(())
        })?;
        self.or_ignore(&account.is_new, |w, value| {
            w.out.bin_write_bool(*value)?;
            Ok(())
        })
    }

    fn preconditions(&mut self, preconditions: &Preconditions) -> Result<()> {
        self.network(&preconditions.network)?;
        self.account(&preconditions.account)?;
        self.global_slot(&preconditions.valid_while)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block::precomputed::{CurrencyEncoding, PcbVersion, PrecomputedBlock},
        command::UserCommandWithStatus,
        mina_blocks::v2::staged_ledger_diff::{UserCommand, UserCommandData},
    };
    use std::path::PathBuf;

    fn zkapp_hashes(
        path: &str,
        encoding: CurrencyEncoding,
    ) -> Result<Vec<(TxnHash, Option<TxnHash>)>> {
        let block = PrecomputedBlock::parse_file(&PathBuf::from(path), PcbVersion::V2(encoding))?;
        let mut hashes = vec![];

        for cmd in block.zkapp_commands() {
            if let UserCommandWithStatus::V2(cmd) = &cmd {
                if let UserCommand {
                    data: (_, UserCommandData::ZkappCommandData(zkapp)),
                    txn_hash,
                    ..
                } = cmd.as_ref()
                {
                    hashes.push((hash_zkapp_command(zkapp)?, txn_hash.clone()));
                }
            }
        }

        Ok(hashes)
    }

    /// Mesa blocks carry no hash. Expected values come from the devnet daemon
    /// and archive-node-api.
    /// Each byte example in the module docs. Change the two places together.
    #[test]
    fn doc_examples() -> Result<()> {
        fn bytes(f: impl FnOnce(&mut Writer) -> Result<()>) -> Result<Vec<u8>> {
            let mut w = Writer { out: vec![] };
            f(&mut w)?;
            Ok(w.out)
        }

        // single-constructor variant
        assert_eq!(bytes(|w| w.slot(5))?, [0x00, 0x05]);

        // unsigned values
        assert_eq!(bytes(|w| w.uint32(40_000))?, [0xfd, 0x40, 0x9c, 0x00, 0x00]);
        assert_eq!(
            bytes(|w| w.uint32(3_000_000_000))?,
            [0xfd, 0x00, 0x5e, 0xd0, 0xb2]
        );
        assert_eq!(bytes(|w| w.uint64(u64::MAX))?, [0xff, 0xff]);

        // the wrap matters: the unwrapped value takes the 8 byte form
        let unwrapped = bytes(|w| {
            w.out.bin_write_integer(3_000_000_000_i64)?;
            Ok(())
        })?;
        assert_eq!(unwrapped[0], 0xfc);
        assert_eq!(unwrapped.len(), 9);

        // string
        assert_eq!(bytes(|w| w.string(b"hi"))?, [0x02, 0x68, 0x69]);

        // vector: elements, then a zero byte
        assert_eq!(
            bytes(|w| {
                w.tag(0xa)?;
                w.tag(0xb)?;
                w.vector_end()
            })?,
            [0x0a, 0x0b, 0x00]
        );
        Ok(())
    }

    #[test]
    fn mesa_block_hashes() -> Result<()> {
        for (path, expected) in [
            // signature authorization only
            (
                "./tests/data/zkapp_hash/devnet-584751-3NLE5RrhKjjMiQLj7uTjLCQC44sL3YnRpE2ZnWiEQq8A7KKNp7zA.json",
                "5JvLDFUfyLHaqp9dLaBCsdMTHJh4nvHYQsD4rmUUtJg1HLaU7zP8",
            ),
            // proof authorization
            (
                "./tests/data/zkapp_hash/devnet-584530-3NLBkWFhcNrMPKUQvkgbPPNnPJmrya9f78vHHvB15kZS9TViL78Q.json",
                "5JuZ7qneVurgyRUDJ5NFdaErqnz9L2yWWkt6fFRYXhaPUR7jYJF4",
            ),
            // zkApp deployment: sets a verification key and permissions
            (
                "./tests/data/zkapp_hash/devnet-581810-3NKs2nZK5amobRZXPQjYB2rnuaERSb3VAQ1DXgBYai6TdHg4LDXJ.json",
                "5Jv4YHHMTHnxZUu6788TYbyrSvoFinK9WQzLfPYVduqg7h1syshd",
            ),
        ] {
            let hashes = zkapp_hashes(path, CurrencyEncoding::DecimalMina)?;
            assert!(hashes.iter().any(|(hash, _)| *hash == TxnHash::V2(expected.to_string())));
        }

        Ok(())
    }

    /// Berkeley blocks embed the protocol's hash
    #[test]
    fn berkeley_block_hashes() -> Result<()> {
        for path in [
            // verification key, permissions and token symbol updates
            "./tests/data/hardfork/mainnet-359617-3NKZ5poCAjtGqg9hHvAVZ7QwriqJsL8mpQsSHFGzqW6ddEEjYfvW.json",
            // a decimal authorization kind hash
            "./tests/data/misc_blocks/mainnet-368442-3NLTFUdvKixsbCqEbjWKskrjWuaSQpwTjoGNXWzK7eaUn4oHscbu.json",
        ] {
            let hashes = zkapp_hashes(path, CurrencyEncoding::Nanomina)?;
            assert!(!hashes.is_empty());

            for (hash, embedded) in hashes {
                assert_eq!(Some(hash), embedded);
            }
        }

        Ok(())
    }
}
