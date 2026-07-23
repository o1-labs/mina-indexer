//! GraphQL time-series chart endpoints.
//!
//! Bucketed counts over the canonical chain for Minascan-style dashboards, the
//! indexer source for Blockberry's `getTransactionsCountChart` /
//! `getZkAppTransactionsCountChart` (issue #95 item 3). Each point is a
//! day / week / month bucket with the number of (user | zkApp) commands in the
//! canonical blocks of that period.
//!
//! Computed query-side (no store schema change): a single scan of the
//! global-slot block index, filtered to canonical blocks, summing the cheap
//! per-block command counts. It is O(blocks) with two point lookups each, so it
//! is served with an hour-long `cache_control` — the same "ship-now, cache it"
//! posture as `timeLocks` and the balance/nonce account paths.
//!
//! Time bucketing derives each block's timestamp from its global slot
//! (`genesis + slot * slot_time`), consistent with the rest of the codebase's
//! slot↔time mapping (`millis_to_global_slot`), which is anchored to the
//! mainnet genesis timestamp.

use crate::{
    block::store::BlockStore,
    command::store::UserCommandStore,
    constants::{from_timestamp_millis, MAINNET_BLOCK_SLOT_TIME_MILLIS, MAINNET_GENESIS_TIMESTAMP},
    utility::store::common::{block_u32_prefix_from_key, state_hash_suffix},
    web::graphql::{db, get_block_canonicity},
};
use async_graphql::{Context, Enum, Object, Result, SimpleObject};
use speedb::IteratorMode;
use std::{collections::BTreeMap, sync::Arc};

/// Bucket granularity for a time-series chart.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ChartBucket {
    Day,
    Week,
    Month,
}

#[derive(SimpleObject)]
pub struct ChartPoint {
    /// Bucket label: `YYYY-MM-DD` (day), `GGGG-Www` ISO week (week), or
    /// `YYYY-MM` (month), UTC. Lexicographic order is chronological.
    pub date: String,

    /// Number of matching commands in the bucket's canonical blocks.
    pub count: u64,
}

/// Which per-block command count a chart sums.
#[derive(Copy, Clone)]
enum CommandKind {
    User,
    Zkapp,
}

/// UTC bucket label for a block at `global_slot`.
fn bucket_key(global_slot: u32, bucket: ChartBucket) -> String {
    let millis = MAINNET_GENESIS_TIMESTAMP as i64
        + global_slot as i64 * MAINNET_BLOCK_SLOT_TIME_MILLIS as i64;
    let dt = from_timestamp_millis(millis);
    match bucket {
        ChartBucket::Day => dt.format("%Y-%m-%d"),
        // %G-W%V = ISO-8601 week-numbering year + week (01..=53).
        ChartBucket::Week => dt.format("%G-W%V"),
        ChartBucket::Month => dt.format("%Y-%m"),
    }
    .to_string()
}

/// Sum the chosen per-block command count over the canonical chain, bucketed by
/// `bucket`. Shared by both chart resolvers so they can't diverge.
fn command_count_chart(
    db: &Arc<crate::store::IndexerStore>,
    bucket: ChartBucket,
    kind: CommandKind,
) -> Result<Vec<ChartPoint>> {
    // BTreeMap keeps buckets in chronological (== lexicographic) key order.
    let mut buckets: BTreeMap<String, u64> = BTreeMap::new();

    for (key, _) in db
        .blocks_global_slot_iterator(IteratorMode::Start)
        .flatten()
    {
        let state_hash = state_hash_suffix(&key)?;

        // Canonical chain only -- the slot index also holds orphaned/pending
        // forks, which would double-count.
        if !get_block_canonicity(db, &state_hash) {
            continue;
        }

        let count = match kind {
            CommandKind::User => db.get_block_user_commands_count(&state_hash)?,
            CommandKind::Zkapp => db.get_block_zkapp_commands_count(&state_hash)?,
        }
        .unwrap_or(0);

        let slot = block_u32_prefix_from_key(&key)?;
        // Record the bucket even when the block is empty, so periods with blocks
        // but no matching commands still appear as a zero point.
        *buckets.entry(bucket_key(slot, bucket)).or_insert(0) += count as u64;
    }

    Ok(buckets
        .into_iter()
        .map(|(date, count)| ChartPoint { date, count })
        .collect())
}

#[derive(Default)]
pub struct ChartsQueryRoot;

#[Object]
impl ChartsQueryRoot {
    /// User-transaction count per `bucket` over the canonical chain, oldest
    /// first. Backs Blockberry's `getTransactionsCountChart`.
    #[graphql(cache_control(max_age = 3600))]
    async fn transactions_count_chart(
        &self,
        ctx: &Context<'_>,
        bucket: ChartBucket,
    ) -> Result<Vec<ChartPoint>> {
        command_count_chart(db(ctx), bucket, CommandKind::User)
    }

    /// zkApp-command count per `bucket` over the canonical chain, oldest first.
    /// Backs Blockberry's `getZkAppTransactionsCountChart`.
    #[graphql(cache_control(max_age = 3600))]
    async fn zkapp_transactions_count_chart(
        &self,
        ctx: &Context<'_>,
        bucket: ChartBucket,
    ) -> Result<Vec<ChartPoint>> {
        command_count_chart(db(ctx), bucket, CommandKind::Zkapp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_key_labels_by_granularity() {
        // Mainnet genesis is 2021-03-17T00:00:00Z (slot 0). 180s slots => slot
        // 480 = +1 day = 2021-03-18.
        assert_eq!(bucket_key(0, ChartBucket::Day), "2021-03-17");
        assert_eq!(bucket_key(0, ChartBucket::Month), "2021-03");
        assert_eq!(bucket_key(480, ChartBucket::Day), "2021-03-18");

        // Same slot maps to a coarser bucket consistently.
        assert_eq!(bucket_key(480, ChartBucket::Month), "2021-03");

        // ISO week label shape: `GGGG-Www`.
        let wk = bucket_key(0, ChartBucket::Week);
        assert!(
            wk.len() == 8 && &wk[4..6] == "-W",
            "unexpected ISO week label: {wk}"
        );
    }

    // Drives both resolvers through the schema against an empty store: no
    // canonical blocks => empty series, no error. Exercises the wiring +
    // canonical filter (every block filtered out) without needing a seeded chain
    // (per-block command counts require full ingest, covered by e2e).
    #[tokio::test]
    async fn charts_empty_store_returns_empty_series() {
        use crate::{store::IndexerStore, web::graphql::build_schema};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let schema = build_schema(store, 0, 0, 0, false);

        for field in ["transactionsCountChart", "zkappTransactionsCountChart"] {
            let q = format!("{{ {field}(bucket: DAY) {{ date count }} }}");
            let res = schema.execute(q).await;
            assert!(res.errors.is_empty(), "{field} errored: {:?}", res.errors);
            let arr = res.data.into_json().unwrap()[field]
                .as_array()
                .unwrap()
                .len();
            assert_eq!(arr, 0, "{field} should be empty on an empty store");
        }
    }
}
