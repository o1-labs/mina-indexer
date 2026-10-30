//! A stop after a block reached the store but before it reached the witness
//! tree and the best tip (kill -9, OOM) must not stop the best tip for good

use crate::helpers::{state::*, store::*};
use mina_indexer::{
    block::{
        parser::BlockParser,
        precomputed::{PcbVersion, PrecomputedBlock},
    },
    constants::*,
    ledger::genesis::GenesisLedger,
    server::IndexerVersion,
    state::{IndexerState, IndexerStateConfig},
};
use std::{fs, path::Path};

#[tokio::test]
async fn stored_but_unapplied_block_is_applied_after_restart() -> anyhow::Result<()> {
    let source = Path::new("./tests/data/canonical_chain_discovery/contiguous");
    let stored = "mainnet-20-3NLPpt5SyVnD1U5uJAqR3DL1Cqj5dG26SuWutRQ6AQpbQtQUWSYA.json";
    let next = "mainnet-21-3NKZ6DTHiMtuaeP3tJq2xe4uujVRnGT9FX1rBiZY521uNToSppUZ.json";

    // ingest every block below 20
    let blocks_dir = tempfile::tempdir()?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() != stored && entry.file_name() != next {
            fs::copy(entry.path(), blocks_dir.path().join(entry.file_name()))?;
        }
    }
    let store_dir = setup_new_db_dir("event-sync-stored-block")?;
    let mut state = mainnet_genesis_state(store_dir.as_ref())?;
    state
        .add_blocks(&mut BlockParser::new_testing(blocks_dir.path())?)
        .await?;
    let store = state.indexer_store.clone().unwrap();

    // block 20 reaches the store, then the process stops
    let parse = |name: &str| -> anyhow::Result<(PrecomputedBlock, u64)> {
        let path = source.join(name);
        Ok((
            PrecomputedBlock::parse_file(&path, PcbVersion::V1)?,
            path.metadata()?.len(),
        ))
    };
    let (block_20, bytes_20) = parse(stored)?;
    let (block_21, bytes_21) = parse(next)?;
    state.add_block_to_store(&block_20, bytes_20, true)?;
    drop(state);

    // restart: sync from the store, then the blocks arrive again
    let config = IndexerStateConfig::new(
        GenesisLedger::new_v1()?,
        IndexerVersion::default(),
        store,
        MAINNET_CANONICAL_THRESHOLD,
        10,
        false,
        false,
    );
    let mut state = IndexerState::new_without_genesis_events(config)?;
    state.sync_from_db()?;
    state.block_pipeline(&block_20, bytes_20)?;
    state.block_pipeline(&block_21, bytes_21)?;

    assert_eq!(state.best_tip_block().state_hash, block_21.state_hash());
    Ok(())
}

/// The sync root is the best chain's block `canonical_threshold` below the best
/// tip, even when the last best tip event at that height was a fork that was
/// reorged away (on restart the root was that fork: every later block dangled
/// and the best tip never advanced again)
#[tokio::test]
async fn sync_roots_at_the_best_chain_not_a_reorged_fork() -> anyhow::Result<()> {
    use mina_indexer::{
        block::store::BlockStore,
        event::{db::*, store::EventStore, IndexerEvent},
    };

    let source = Path::new("./tests/data/canonical_chain_discovery/contiguous");
    let next = "mainnet-21-3NKZ6DTHiMtuaeP3tJq2xe4uujVRnGT9FX1rBiZY521uNToSppUZ.json";
    let fork = Path::new("./tests/data/canonical_chain_discovery/gaps/mainnet-10-3NKHYHrqKpDcon6ToV5CLDiheanjshk5gcsNqefnK78phCFTR2aL.json");

    // the best chain to 20
    let blocks_dir = tempfile::tempdir()?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_name() != next {
            fs::copy(entry.path(), blocks_dir.path().join(entry.file_name()))?;
        }
    }
    let store_dir = setup_new_db_dir("event-sync-root-fork")?;
    let mut state = mainnet_genesis_state(store_dir.as_ref())?;
    state
        .add_blocks(&mut BlockParser::new_testing(blocks_dir.path())?)
        .await?;
    let store = state.indexer_store.clone().unwrap();
    let best = state.best_tip_block().clone();
    drop(state);

    // a sibling at the sync root height (best - 10) was the last best tip
    // there before the best chain went on
    let fork_block = PrecomputedBlock::parse_file(fork, PcbVersion::V1)?;
    store.add_block(&fork_block, fork.metadata()?.len())?;
    store.add_event(&IndexerEvent::Db(DbEvent::Block(DbBlockEvent::NewBestTip {
        state_hash: fork_block.state_hash(),
        blockchain_length: fork_block.blockchain_length(),
    })))?;
    assert_eq!(best.blockchain_length - MAINNET_CANONICAL_THRESHOLD, fork_block.blockchain_length());

    // restart
    let config = IndexerStateConfig::new(
        GenesisLedger::new_v1()?,
        IndexerVersion::default(),
        store,
        MAINNET_CANONICAL_THRESHOLD,
        10,
        false,
        false,
    );
    let mut state = IndexerState::new_without_genesis_events(config)?;
    state.sync_from_db()?;
    assert_eq!(state.best_tip_block().state_hash, best.state_hash);
    assert!(state.dangling_branches.is_empty());

    // and the next block extends it
    let path = source.join(next);
    let block_21 = PrecomputedBlock::parse_file(&path, PcbVersion::V1)?;
    state.block_pipeline(&block_21, path.metadata()?.len())?;
    assert_eq!(state.best_tip_block().state_hash, block_21.state_hash());
    Ok(())
}
