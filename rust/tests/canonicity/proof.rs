use crate::helpers::{state::*, store::*};
use mina_indexer::{
    base::state_hash::StateHash,
    block::{parser::BlockParser, store::BlockStore},
    canonicity::{proof::build_canonicity_proof, store::CanonicityStore},
};
use std::path::PathBuf;

/// Tier-1 canonicity proof over the 20-block contiguous canonical chain: a
/// canonical target yields a valid, parent-linked chain up to a canonical
/// anchor; a non-canonical/unknown target yields no proof.
#[tokio::test]
async fn canonicity_proof_over_contiguous_chain() -> anyhow::Result<()> {
    let store_dir = setup_new_db_dir("canonicity-proof")?;
    let block_dir = PathBuf::from("./tests/data/canonical_chain_discovery/contiguous");

    let mut block_parser = BlockParser::new_testing(&block_dir)?;
    let mut state = mainnet_genesis_state(store_dir.as_ref())?;
    state.add_blocks(&mut block_parser).await?;

    let store = state.indexer_store.as_ref().unwrap();
    let best_tip_height = store.get_best_block_height()?.unwrap();
    // The 20-block contiguous fixture builds a chain deep enough that the target
    // + anchor window below sits inside the canonical (finalized) zone.
    assert!(best_tip_height >= 15, "fixture chain too short: {best_tip_height}");

    // Target a deep (finalized) canonical block; anchor 4 blocks above it — all
    // within the canonical zone (canonical extends to ~tip − threshold).
    let target_height = 5u32;
    let anchor_span = 4u32;
    let target = store.get_canonical_hash_at_height(target_height)?.unwrap();

    let proof = build_canonicity_proof(store, &target, anchor_span)?
        .expect("a canonical target must produce a proof");

    // Shape.
    assert_eq!(proof.target_height, target_height);
    assert_eq!(proof.best_tip_height, best_tip_height);
    assert_eq!(proof.depth, best_tip_height - target_height);
    assert_eq!(proof.parent_chain.len(), (anchor_span + 1) as usize);

    // The chain is exactly the canonical hashes at heights target..=target+span,
    // starting at the target and ending at the anchor.
    assert_eq!(proof.parent_chain.first(), Some(&target));
    for (i, hash) in proof.parent_chain.iter().enumerate() {
        let expected = store
            .get_canonical_hash_at_height(target_height + i as u32)?
            .unwrap();
        assert_eq!(hash, &expected, "chain entry {i} is the canonical hash");
    }
    assert_eq!(
        proof.anchor_state_hash,
        *proof.parent_chain.last().unwrap()
    );

    // Parent linkage: each entry's parent hash is its predecessor in the chain
    // (this is what the client re-checks).
    for pair in proof.parent_chain.windows(2) {
        let parent = store.get_block_parent_hash(&pair[1])?.unwrap();
        assert_eq!(parent, pair[0], "each block links to its canonical parent");
    }

    // Fail closed: an unknown/orphaned state hash gets no proof.
    let bogus = StateHash("3NnotARealBlockHashnotARealBlockHashnotARealBlock00".to_string());
    assert!(build_canonicity_proof(store, &bogus, anchor_span)?.is_none());

    // A large span caps at the top of the canonical prefix (never past it) and
    // still links cleanly.
    let wide = build_canonicity_proof(store, &target, 1000)?.unwrap();
    assert!(wide.parent_chain.len() >= (anchor_span + 1) as usize);
    for pair in wide.parent_chain.windows(2) {
        assert_eq!(store.get_block_parent_hash(&pair[1])?.unwrap(), pair[0]);
    }

    Ok(())
}
