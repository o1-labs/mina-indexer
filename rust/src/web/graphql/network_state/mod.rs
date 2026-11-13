//! GraphQL `networkState` endpoint
//!
//! It gives two heights:
//! - `pendingMaxBlockHeight`: the highest block (the best tip)
//! - `canonicalMaxBlockHeight`: the highest final block. A block is final when
//!   it leaves the transition frontier, k blocks below the best tip.
//!
//! The indexer marks every best-chain block canonical, up to the best tip, so
//! its canonical flag is not used here: the final height is `best tip - k`,
//! never below the network's genesis block.

use super::db;
use crate::{
    block::store::BlockStore, constants::MAINNET_TRANSITION_FRONTIER_K, store::IndexerStore,
};
use async_graphql::{Context, Object, SimpleObject};

#[derive(Default)]
pub struct NetworkStateQueryRoot;

#[derive(SimpleObject, Debug, PartialEq, Eq)]
#[graphql(name = "NetworkStateOutput")]
pub struct NetworkState {
    max_block_height: MaxBlockHeightInfo,
}

#[derive(SimpleObject, Debug, PartialEq, Eq)]
#[graphql(name = "MaxBlockHeightInfo")]
pub struct MaxBlockHeightInfo {
    /// Highest final block (k blocks below the best tip)
    canonical_max_block_height: u32,

    /// Highest block (the best tip)
    pending_max_block_height: u32,
}

#[Object]
impl NetworkStateQueryRoot {
    /// Best tip and final block heights
    async fn network_state(&self, ctx: &Context<'_>) -> anyhow::Result<NetworkState> {
        network_state(db(ctx))
    }
}

fn network_state(db: &IndexerStore) -> anyhow::Result<NetworkState> {
    // no best block yet (empty store, before the first block is ingested)
    // gives height 0
    let pending = db.get_best_block_height()?.unwrap_or(0);

    // the genesis (or fork root) block is final from the start
    let genesis = match db.get_best_block_genesis_hash()? {
        Some(hash) => db.get_block_height(&hash)?.unwrap_or(0),
        None => 0,
    };

    Ok(NetworkState {
        max_block_height: MaxBlockHeightInfo::new(pending, genesis),
    })
}

impl MaxBlockHeightInfo {
    /// Heights for the best tip `pending` of a network that starts at `genesis`
    fn new(pending: u32, genesis: u32) -> Self {
        let canonical = pending
            .saturating_sub(MAINNET_TRANSITION_FRONTIER_K)
            .max(genesis)
            .min(pending);

        Self {
            canonical_max_block_height: canonical,
            pending_max_block_height: pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heights(pending: u32, genesis: u32) -> (u32, u32) {
        let info = MaxBlockHeightInfo::new(pending, genesis);
        (
            info.canonical_max_block_height,
            info.pending_max_block_height,
        )
    }

    #[test]
    fn canonical_is_k_below_the_best_tip() {
        // live devnet values
        assert_eq!(heights(589_131, 545_434), (588_841, 589_131));
    }

    #[test]
    fn canonical_is_never_below_genesis() {
        assert_eq!(heights(545_500, 545_434), (545_434, 545_500));
        assert_eq!(heights(545_434, 545_434), (545_434, 545_434));
        assert_eq!(heights(100, 1), (1, 100));
    }

    #[test]
    fn empty_store_gives_zero() {
        assert_eq!(heights(0, 0), (0, 0));
    }

    /// Run the query through the full GraphQL schema on an empty store and check
    /// the type names (`__typename`), the field names and the heights (0, as
    /// there is no best block). Clients that select these names must not break.
    #[tokio::test]
    async fn query_has_the_expected_shape() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = std::sync::Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let schema = crate::web::graphql::build_schema(store, 0, 0, 0, false);

        let resp = schema
            .execute(
                "{ networkState { __typename maxBlockHeight { __typename \
                 canonicalMaxBlockHeight pendingMaxBlockHeight } } }",
            )
            .await;
        assert!(resp.errors.is_empty(), "{:?}", resp.errors);
        assert_eq!(
            resp.data.into_json().unwrap(),
            serde_json::json!({ "networkState": {
                "__typename": "NetworkStateOutput",
                "maxBlockHeight": {
                    "__typename": "MaxBlockHeightInfo",
                    "canonicalMaxBlockHeight": 0,
                    "pendingMaxBlockHeight": 0,
                }
            }})
        );
    }
}
