//! End-to-end test for `--network-config` (custom networks).
//!
//! Drives the same path the binary takes for a custom network: parse the
//! descriptor, build the `IndexerVersion` from it, then `new_from_config`. The
//! point is that the runtime genesis quantities from the descriptor are
//! persisted as a [`ConfigGenesis`] record (the store impls read this instead
//! of the hardcoded `*_GENESIS_*` constants) and the network/chain-id resolve
//! from the descriptor rather than the genesis-hash dispatch.
//!
//! The descriptor's `genesis_block` is the real hardfork fork block, whose
//! staged-ledger diff is empty by construction (a custom-network genesis block
//! applies as a no-op onto the runtime-supplied ledger, like the embedded
//! mesa/devnet ones). That decouples the test from the genesis ledger, so we
//! point `genesis_ledger` at the small mainnet ledger to keep the test fast —
//! the persisted `ConfigGenesis` is the descriptor's declared genesis,
//! independent of the ledger file's contents.

use crate::helpers::store::setup_new_db_dir;
use mina_indexer::{
    base::state_hash::StateHash,
    chain::{
        store::{ChainStore, ConfigGenesis},
        ChainId, Network, NetworkConfig,
    },
    constants::{MAINNET_CANONICAL_THRESHOLD, MAINNET_TRANSITION_FRONTIER_K},
    ledger::{genesis::GenesisLedger, LedgerHash},
    server::IndexerVersion,
    state::{IndexerState, IndexerStateConfig},
    store::IndexerStore,
};
use std::{path::PathBuf, sync::Arc};

/// Booting from the `network.json` fixture persists its runtime genesis and
/// resolves the network/chain id from the descriptor.
#[test]
fn config_genesis_persisted_and_network_resolved_from_descriptor() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("config-genesis")?;
    let store = Arc::new(IndexerStore::new(store_dir.as_ref(), true)?);

    // Parse the custom-network descriptor (relative genesis ledger/block paths
    // resolve against the descriptor's parent dir).
    let descriptor = PathBuf::from("./tests/data/network_config/network.json");
    let network_config = NetworkConfig::parse_file(&descriptor)?;

    // Build the version from the descriptor and load the genesis ledger it
    // points at — mirrors `bin/mina-indexer.rs`.
    let version = IndexerVersion::from_config(&network_config)?;
    let genesis_ledger = GenesisLedger::parse_file(
        network_config
            .genesis_ledger
            .as_ref()
            .expect("descriptor supplies a genesis ledger"),
    )?;

    let mut config = IndexerStateConfig::new(
        genesis_ledger,
        version,
        store.clone(),
        MAINNET_CANONICAL_THRESHOLD,
        MAINNET_TRANSITION_FRONTIER_K,
        false,
        false,
    );
    config.network_config = Some(network_config);

    let _state = IndexerState::new_from_config(config)?;

    // The runtime genesis from the descriptor must be persisted verbatim.
    let persisted = store
        .get_config_genesis()?
        .expect("custom network persists a ConfigGenesis record");
    assert_eq!(
        persisted,
        ConfigGenesis {
            state_hash: StateHash::from("3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP"),
            prev_state_hash: StateHash::from(
                "3NLRTfY4kZyJtvaP4dFenDcxfoMfT3uEpkWS913KkeXLtziyVd15"
            ),
            ledger_hash: LedgerHash::new_or_panic(
                "jwNw4qb6tnNhpQNxiMLem9WumxZTwmbSx3fYXW4FP3hZRkoQJSE".to_string()
            ),
            blockchain_length: 359605,
        }
    );

    // Network and chain id resolve from the descriptor, not the hardcoded
    // genesis-hash dispatch ("testnet" => Network::Custom).
    assert_eq!(store.get_current_network()?, Network::from("testnet"));
    assert_eq!(
        store.get_current_network()?,
        Network::Custom("testnet".to_string())
    );
    assert_eq!(
        store.get_chain_id()?,
        ChainId::from_config("6d6573612d6d75740000000000000000000000000000000000000000000000aa")?
    );

    Ok(())
}
