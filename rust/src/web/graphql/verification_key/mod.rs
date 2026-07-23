//! GraphQL `verificationKeyHistory` / `lastVerificationKeyChange` endpoints.
//!
//! Serves a zkApp account's verification-key change log from the height-ordered
//! `zkapp-verification-key-history` index (issue #95 item 5). Backs
//! Blockberry's `getVerificationKeyHistory` / `getLastVerificationKeyChange`.
//!
//! Only *actual* changes are indexed (see `VerificationKeyChange`): the first
//! key set is the VK's creation (`old_verification_key_hash: null`), and no-op
//! re-sets to the same hash are not recorded. Entries are canonical-filtered at
//! query time (the index also holds records from orphaned blocks),
//! newest-first.

use super::{
    accounts::zkapp::VerificationKey, date_time::DateTime, date_time_to_scalar, db, get_block,
    get_block_canonicity,
};
use crate::{
    base::public_key::PublicKey,
    store::zkapp::{VerificationKeyChange, ZkappStore},
    utility::store::common::{from_be_bytes, state_hash_suffix, U32_LEN},
};
use async_graphql::{Context, Object, Result, SimpleObject};
use speedb::Direction;

#[derive(SimpleObject)]
pub struct VerificationKeyChangeEntry {
    /// Block height the change was applied at.
    #[graphql(name = "block_height")]
    pub block_height: u32,

    /// State hash of that block.
    #[graphql(name = "state_hash")]
    pub state_hash: String,

    /// Block timestamp.
    #[graphql(name = "date_time")]
    pub date_time: DateTime,

    /// Hash of the transaction that set the key.
    #[graphql(name = "txn_hash")]
    pub txn_hash: String,

    /// Token of the zkApp account.
    pub token: String,

    /// Hash of the previous verification key, or `null` when this is the first
    /// key set on the account (its creation).
    #[graphql(name = "old_verification_key_hash")]
    pub old_verification_key_hash: Option<String>,

    /// The verification key set by this change (full key + hash).
    #[graphql(name = "verification_key")]
    pub verification_key: VerificationKey,
}

/// Build a GraphQL entry from an index record. The block timestamp comes from
/// the block itself (VK changes are rare, so the per-entry block read is
/// cheap).
fn to_entry(
    db: &std::sync::Arc<crate::store::IndexerStore>,
    block_height: u32,
    state_hash: crate::base::state_hash::StateHash,
    change: VerificationKeyChange,
) -> VerificationKeyChangeEntry {
    let date_time = date_time_to_scalar(get_block(db, &state_hash).timestamp() as i64);
    VerificationKeyChangeEntry {
        block_height,
        state_hash: state_hash.0,
        date_time,
        txn_hash: change.txn_hash.inner(),
        token: change.token.0,
        old_verification_key_hash: change.old_vk_hash.map(|h| h.0),
        verification_key: change.verification_key.into(),
    }
}

#[derive(Default)]
pub struct VerificationKeyQueryRoot;

#[Object]
impl VerificationKeyQueryRoot {
    /// A zkApp account's verification-key change history, newest first,
    /// canonical only. Backs Blockberry's `getVerificationKeyHistory`.
    async fn verification_key_history(
        &self,
        ctx: &Context<'_>,
        address: String,
        #[graphql(default = 100)] limit: usize,
        // `offset`: matching changes to skip before `limit` -- pages the history.
        #[graphql(default = 0)] offset: usize,
    ) -> Result<Vec<VerificationKeyChangeEntry>> {
        let db = db(ctx);
        let limit = limit.min(crate::constants::GRAPHQL_MAX_PAGE_SIZE);
        let pk = PublicKey::new(&address)
            .map_err(|_| async_graphql::Error::new(format!("Invalid public key: {address}")))?;

        let mut out = Vec::new();
        let mut skipped = 0;
        for (key, value) in db
            .zkapp_verification_key_history_iterator(&pk, Direction::Reverse)
            .flatten()
        {
            if out.len() >= limit {
                break;
            }
            // stop once the scan leaves this pk's contiguous range
            if key.len() < PublicKey::LEN || key[..PublicKey::LEN] != *pk.0.as_bytes() {
                break;
            }

            let state_hash = state_hash_suffix(&key)?;
            if !get_block_canonicity(db, &state_hash) {
                continue;
            }
            if skipped < offset {
                skipped += 1;
                continue;
            }

            let block_height = from_be_bytes(key[PublicKey::LEN..][..U32_LEN].to_vec());
            let change: VerificationKeyChange = serde_json::from_slice(&value)?;
            out.push(to_entry(db, block_height, state_hash, change));
        }

        Ok(out)
    }

    /// The most recent canonical verification-key change for a zkApp account,
    /// or `null` if the key never changed. Backs
    /// `getLastVerificationKeyChange`.
    async fn last_verification_key_change(
        &self,
        ctx: &Context<'_>,
        address: String,
    ) -> Result<Option<VerificationKeyChangeEntry>> {
        let db = db(ctx);
        let pk = PublicKey::new(&address)
            .map_err(|_| async_graphql::Error::new(format!("Invalid public key: {address}")))?;

        // reverse scan -> first canonical record is the most recent change
        for (key, value) in db
            .zkapp_verification_key_history_iterator(&pk, Direction::Reverse)
            .flatten()
        {
            if key.len() < PublicKey::LEN || key[..PublicKey::LEN] != *pk.0.as_bytes() {
                break;
            }
            let state_hash = state_hash_suffix(&key)?;
            if !get_block_canonicity(db, &state_hash) {
                continue;
            }
            let block_height = from_be_bytes(key[PublicKey::LEN..][..U32_LEN].to_vec());
            let change: VerificationKeyChange = serde_json::from_slice(&value)?;
            return Ok(Some(to_entry(db, block_height, state_hash, change)));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use crate::{store::IndexerStore, web::graphql::build_schema};
    use std::sync::Arc;
    use tempfile::TempDir;

    // Empty store => empty history and a null last-change for a valid key; a
    // malformed address is rejected. The populated path (records written by the
    // ingest hook) is covered by integration.
    #[tokio::test]
    async fn vk_history_empty_store_and_bad_address() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let schema = build_schema(store, 0, 0, 0, false);
        let addr = "B62qmK2RecMoNXcqvt6K9k7yKG81qhyMoXhCfZ15SXNa5ikJaJr3urk";

        let hist = schema
            .execute(format!(
                "{{ verificationKeyHistory(address: \"{addr}\") {{ block_height }} }}"
            ))
            .await;
        assert!(
            hist.errors.is_empty(),
            "unexpected error: {:?}",
            hist.errors
        );
        assert_eq!(
            hist.data.into_json().unwrap()["verificationKeyHistory"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let last = schema
            .execute(format!(
                "{{ lastVerificationKeyChange(address: \"{addr}\") {{ block_height }} }}"
            ))
            .await;
        assert!(
            last.errors.is_empty(),
            "unexpected error: {:?}",
            last.errors
        );
        assert!(last.data.into_json().unwrap()["lastVerificationKeyChange"].is_null());

        let bad = schema
            .execute("{ lastVerificationKeyChange(address: \"nope\") { block_height } }")
            .await;
        assert!(!bad.errors.is_empty(), "bad address should error");
    }
}
