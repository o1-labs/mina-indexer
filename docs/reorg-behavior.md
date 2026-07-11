# Reorg behavior & query finality

What the indexer does when the chain reorganizes, how deep a reorg can reach,
and — the practical upshot — **which query results are final and which are
provisional**. For how the canonical chain is first discovered at build time see
[`canonical-chain-discovery.md`](canonical-chain-discovery.md).

## The witness tree

The indexer keeps recent chain history in an in-memory **witness tree**: a tree of
blocks rooted at the *witness root*, with one or more branches toward the tip.
Competing branches (two blocks at the same height) coexist in the tree until one
wins. The tree spans at most the **transition frontier depth `k` = 290** blocks
(`MAINNET_TRANSITION_FRONTIER_K`); everything older is committed to the speedb
store and dropped from the tree.

The **best tip** is the leaf of the best branch. A **reorg** is the best tip
moving from one branch to another — which re-labels the canonicity of the blocks
between the old and new tips.

## How the best chain is chosen

Best-tip selection follows Mina's consensus `selectLongerChain`
([spec §6.2](https://github.com/MinaProtocol/mina/tree/develop/docs/specs/consensus#62-select-chain)),
implemented in `Block::cmp` (`block/mod.rs`). Given two candidate tips, the winner
is decided in order by:

1. **Hardfork era** — a post-hardfork (V2) block always beats a pre-hardfork (V1) one.
2. **Chain length** — longer wins.
3. **Last VRF output hash** — higher `hash_last_vrf_output` wins.
4. **State hash** — higher `state_hash` wins (final deterministic tiebreak).

A new block triggers a reorg only when it (or the branch it extends) is *better*
than the current best tip by this ordering — most commonly by being longer.

## Finality zones (why some answers are provisional)

Measured as depth below the best tip (`d = best_tip_height − block_height`), with
the defaults **canonical threshold = 10** (`MAINNET_CANONICAL_THRESHOLD`,
`--canonical-threshold`) and **k = 290**:

| Zone | Depth | Canonicity | Can a reorg change it? |
|---|---|---|---|
| **Pending** | `d < 10` | not yet `Canonical` | **Yes** — routinely. These are the tip and its most recent ancestors; a competing branch can still win. |
| **Canonical (in-tree)** | `10 ≤ d < 290` | `Canonical` | In principle yes, but only a competing branch that deep could flip it — which does not happen in practice on mainnet. Treat as effectively final. |
| **Committed** | `d ≥ 290` | `Canonical`, pruned to the store | **No** — beyond the witness tree; immutable. No reorg can reach here. |

So a block becomes `Canonical` once it is **10 blocks deep**, and becomes
*permanent* once it is **290 blocks deep**. The 10-deep mark mirrors Mina's own
soft-finality convention; the 290-deep mark is the transition-frontier hard bound.

## What an operator observes during a reorg

- The **canonicity of a recent block can flip** (`Canonical` ↔ `Pending`/orphaned)
  until it is past the canonical threshold. A block's `canonical` field and any
  canonical-chain query are *provisional* inside the pending zone.
- `mina_indexer_dangling_branches` (Prometheus) is **> 0** while disconnected /
  competing branches exist in the tree; a persistently high value can mean missing
  parents rather than a benign reorg (see the alert in [`ops/observability/`](../ops/observability/README.md)).
- `mina_indexer_best_tip_height` can briefly **stall or step back** at the moment
  the tip switches branches, then resume.
- Nothing at depth `≥ 290` ever changes.

## Guidance for consumers (wallets / L2s / explorers)

- **Treat blocks and transactions within the canonical threshold (10) as
  non-final.** Do not report a recent transaction as settled until it is at least
  10 blocks deep; for high-value flows, wait deeper.
- Query a block's `canonical` flag rather than assuming inclusion is permanent.
- The indexer never *rewrites* committed history (`d ≥ 290`), so anything you read
  from that zone is stable across restarts and reorgs.

## Deep-reorg safety & testing

- A reorg deeper than the canonical threshold is representable in the tree (up to
  `k = 290`) but is not expected on mainnet; a reorg deeper than `k` is **not
  representable** — those blocks are already committed. If upstream ever produced
  such a fork, recovery is a re-index (see [`disaster-recovery.md`](disaster-recovery.md)),
  not an in-place rewrite.
- The branch/best-tip machinery is exercised by the witness-tree suites under
  `rust/tests/state/` (`dangling_branches/`, `root_branch/`), which build competing
  branches and assert tree extension, merging, and pruning.
