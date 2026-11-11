//! Tier-1 trustless **canonicity proof**.
//!
//! The material a client re-checks to confirm a block is on the canonical chain
//! (not a valid-but-orphaned fork), anchored at a recent canonical block the
//! client independently trusts (e.g. from its own light-node tip). The indexer
//! is untrusted: it only *assembles* this material from its canonical-chain
//! index; the client re-verifies the parent linkage and that the anchor is a tip
//! it already trusts. See `docs/trustless-responses.md` (Tier 1).
//!
//! Correctness boundary: an orphaned/unknown block must **never** be presentable
//! as "the block at height H". [`build_canonicity_proof`] returns `Ok(None)` for
//! any target that is not the canonical block at its own height, or whose chain
//! doesn't link — it never emits a fabricated or broken proof.

use crate::{
    base::state_hash::StateHash, block::store::BlockStore, canonicity::store::CanonicityStore,
    store::IndexerStore,
};
use anyhow::Context;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CanonicityProof {
    /// Height of the target block.
    pub target_height: u32,

    /// Current best-tip height (lets the client apply k-finality via `depth`).
    pub best_tip_height: u32,

    /// Depth of the target below the best tip (`best_tip_height - target_height`).
    pub depth: u32,

    /// A recent canonical block the client is expected to independently trust —
    /// the top (highest) entry of `parent_chain`.
    pub anchor_state_hash: StateHash,

    /// Canonical state-hash chain from the target (inclusive, `[0]`) up to the
    /// anchor (inclusive, last), ascending by height. Consecutive entries link by
    /// parent hash: the parent hash of `parent_chain[i + 1]` equals
    /// `parent_chain[i]`. The client re-checks the linkage and that the top is a
    /// tip it trusts, proving the target is a canonical ancestor of that tip.
    pub parent_chain: Vec<StateHash>,
}

/// Build a canonicity proof for `target`, or `Ok(None)` if `target` is not the
/// canonical block at its height (orphaned / unknown), or its canonical chain
/// has a hole or broken linkage — fail closed, never emit a bad proof.
///
/// `anchor_span` bounds how far above the target the emitted chain reaches; the
/// walk also stops at the top of the canonical prefix (canonicity only extends
/// to roughly `best_tip − canonical_threshold`, so heights nearer the tip have no
/// canonical block yet).
pub fn build_canonicity_proof(
    db: &IndexerStore,
    target: &StateHash,
    anchor_span: u32,
) -> anyhow::Result<Option<CanonicityProof>> {
    let Some(target_height) = db.get_block_height(target)? else {
        return Ok(None);
    };
    let best_tip_height = db.get_best_block_height()?.context("no best block height")?;

    let max_height = target_height
        .saturating_add(anchor_span)
        .min(best_tip_height);

    let mut parent_chain: Vec<StateHash> = Vec::new();
    for height in target_height..=max_height {
        let Some(hash) = db.get_canonical_hash_at_height(height)? else {
            // Reached the top of the canonical prefix — stop, don't refuse.
            break;
        };

        // The target must BE the canonical block at its own height, else it is
        // orphaned and gets no proof.
        if height == target_height && &hash != target {
            return Ok(None);
        }

        // Verify parent linkage against the previous canonical hash. We assert it
        // here so a broken canonical index never yields a chain that won't verify
        // client-side.
        if let Some(prev) = parent_chain.last() {
            match db.get_block_parent_hash(&hash)? {
                Some(parent) if &parent == prev => {}
                _ => return Ok(None),
            }
        }

        parent_chain.push(hash);
    }

    // At minimum the target itself must be present and canonical.
    if parent_chain.first() != Some(target) {
        return Ok(None);
    }

    let anchor_state_hash = parent_chain
        .last()
        .cloned()
        .expect("parent_chain non-empty (checked above)");

    Ok(Some(CanonicityProof {
        target_height,
        best_tip_height,
        depth: best_tip_height - target_height,
        anchor_state_hash,
        parent_chain,
    }))
}
