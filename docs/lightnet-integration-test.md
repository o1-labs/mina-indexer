# Scoping: node → indexer live integration test (lightnet)

Status: **proposal / scoping** — not yet built. Owner: TBD. Tracking issue: TBD.

## Summary

Stand up a hermetic integration test in which a **local Mina network produces
blocks**, the **indexer ingests them live**, and we assert the indexer's chain
and ledger match the **node's own view**. The node is used as an independent
oracle, which no current test does.

The one-line justification: **this is the test that would have caught
[#119](https://github.com/o1-labs/mina-indexer/pull/119)** — a reconcile bug that
reached production and that no existing test exercises.

## Why — and, honestly, why *not* for correctness

It is tempting to pitch this as "prove the indexer's ledger is correct against a
real node." That would be **largely redundant**: the indexer's ledger is already
validated against the *cryptographically authoritative* ledger. The existing
fidelity checks assert every balance against the block's `accounts_accessed`,
and we have proven that the indexer's reconstructed ledger **hashes to the
block's proof-covered `staged_ledger_hash`** (see `docs/ledger-calculations.md`
and the fidelity harness). A node's exported ledger is not *more* authoritative
than the SNARK-proved root.

So the non-redundant value of a live-node test is **not ledger arithmetic** — it
is the **live pipeline**:

- **ingest → reconcile → reorg** under a real, moving block stream;
- **dynamic genesis** (a freshly generated local-network genesis, not the
  embedded one);
- **gaps / ordering / timing** of blocks arriving over time;
- **reorg convergence** — the indexer settling on the node's canonical tip after
  short forks.

`#119` (the reconcile loop re-applying store-present blocks) lived exactly here
and slipped every gate. That class of bug is what this test protects.

## Where it fits in the test landscape

| Layer | Exists? | Determinism | Oracle | Catches |
|---|---|---|---|---|
| Unit (in-crate) | ✅ | hermetic | self | logic bugs |
| Static replay e2e (`tests/e2e/fidelity.sh`) | ✅ | hermetic | in-block `accounts_accessed` (== proof-covered root) | ledger-math regressions on canned blocks |
| Live conformance (`compare.yml`) | ✅ | live-vs-live | archive-node-api | real drift (but can't tell which side is wrong; flaky on archive outages) |
| **Live node → indexer (this doc)** | ❌ | hermetic | the node itself | **live ingest/reconcile/reorg pipeline** |

## Architecture

The indexer needs **no changes** — it already ingests by polling a
`--blocks-dir` (globs `*-*-*.json`). This test slots into the exact shape of
`tests/e2e/fidelity.sh`, swapping the block *source* and the *oracle*:

```
mina-local-network.sh                         mina-indexer
  (2 whales, proof-level none,                  server start
   --override-slot-time 1000)                     --blocks-dir <DIR>
        │                                          --genesis-hash <generated>
        │ --precomputed-blocks-file                --genesis-ledger <generated>
        ▼                                                 ▲
  precomputed_blocks.log                                  │ globs *-*-*.json
  (append-only, 1 JSON/line,                              │
   NO state_hash field)                                   │
        │                                          <DIR>/net-<h>-<hash>.json
        │  naming step (see below)  ─────────────────────►
        │
        ▼  compare
  node oracle:  bestChain / account / `mina ledger export staged-ledger`
        └──────────────  assert  ──────────────►  indexer GraphQL:
                                                    bestChain, stagedLedgerAccounts,
                                                    ledger hash
```

### The one non-obvious problem: naming the blocks

The daemon's `--precomputed-blocks-file`
(`src/app/cli/src/cli_entrypoint/mina_cli_entrypoint.ml:505-514`) appends a
single log file, **one JSON object per line**
(`src/lib/mina_lib/mina_subscriptions.ml:121-258`). The precomputed record
(`src/lib/mina_block/precomputed_block.ml:52-63`) carries height
(`protocol_state.body.consensus_state.blockchain_length`) but **no
`state_hash`** — which the indexer's `<network>-<height>-<hash>.json` filename
contract requires.

The repo's own splitter (`scripts/mina-local-network/split_precomputed_log.sh`)
recovers the hash via a **postgres archive-DB lookup**, which would pull a full
`daemon + archive + postgres` stack into the test.

**Simplification (recommended): recover the hash from GraphQL, not postgres.**
The daemon's `newBlock` subscription / `bestChain` query returns
`(blockHeight, stateHash)` pairs directly. A small mapper watches those and names
each log line by height, emitting `net-<height>-<stateHash>.json`. This keeps the
stack to **daemon + indexer + a naming script** — no archive, no postgres.

### Dynamic genesis wiring

`mina-local-network.sh` generates a fresh genesis (random keys, `date`-based
timestamp) each run. The indexer must be pointed at *that* generated genesis
ledger + hash (`--genesis-ledger` / `--genesis-hash`), not an embedded one —
unlike the `mainnet-e2e` fixture which uses the embedded hardfork genesis. The
launcher writes the generated genesis ledger to disk; the test reads its hash and
threads both into the indexer.

## Node-side building blocks (evidence)

| Need | Mechanism | Location |
|---|---|---|
| Emit precomputed blocks | `--precomputed-blocks-file PATH` (append-only log) | `mina_cli_entrypoint.ml:505-514`; writer `mina_subscriptions.ml:121-258` |
| Local block-producing net | `scripts/mina-local-network/mina-local-network.sh` (already wires the flag at L335) | BP keys L1018/1073/1084; genesis L453; slot-time L938-940; proof-level default `full` L36 |
| Fast, cheap blocks | `--proof-level none` + `-st/--override-slot-time 1000` | script L36, L626-627 |
| Whole-ledger oracle | `mina ledger export staged-ledger [--state-hash H]` | `src/app/cli/src/init/client.ml:774-839` |
| Ledger-hash oracle | `mina ledger hash --ledger-file <exported.json>` | `client.ml:841-880` |
| Per-account oracle | GraphQL `account(publicKey:) { balance { total liquid } }` | daemon REST/GraphQL port |

## Assertions

1. **Chain agreement** — indexer `bestChain` state hashes == node `bestChain`
   (below the tip by a settling margin, as the fidelity harness does).
2. **Ledger hash** — indexer's computed ledger hash at a settled height == node's
   `mina ledger hash` of the exported staged ledger at the same state hash.
3. **Account spot checks** — a sample of `account.balance` matches between
   indexer GraphQL and node GraphQL.
4. **Reorg convergence** — with 2 whales, short forks occur naturally; assert the
   indexer converges to the node's canonical tip after each (this is the
   highest-value, hardest-to-get assertion and the direct #119 analogue).

## Staged plan

- **Tier A — happy-path live ingest (nightly).** Local net (proof-level none,
  short slots) → GraphQL-named blocks → indexer; assertions 1–3 at a settled
  height. Est. **~2–4 days**.
- **Tier B — reorg convergence.** Add assertion 4 over the natural 2-whale forks;
  optionally induce a partition to force deeper reorgs. Est. **+1–2 days**.

Not a PR gate — daemon cold-start (genesis proof, tens of seconds) makes it a
**nightly / `Long`-tagged** job.

## Risks / open questions

1. **CI infra — the main risk.** The job needs a prebuilt `mina.exe` daemon image
   (building OCaml in-job is too heavy). Devnet runs one; confirm we can pull it
   in CI. *If no image is available, sourcing it is the first task.*
2. **Cold-start weight.** Tens of seconds to first block even at `proof-level
   none`; producing 10–20 blocks for a meaningful assertion adds ~1–2 min. Budget
   the nightly accordingly.
3. **Naming-script correctness.** The GraphQL-based hash mapper is new code and
   must handle the log/subscription race (a block logged before its stateHash is
   observed). Buffer + reconcile by height.
4. **Flakiness discipline.** Any live-network test drifts toward flaky; keep
   assertions below a settling margin and give the network a bounded warm-up
   (`--slot-chain-end` to stop after N slots for determinism).

## Non-goals

- Not a ledger-correctness check (already covered by fidelity/e2e against the
  proof-covered root).
- Not a performance benchmark (that's `ops/bench/`).
- Not a replacement for `compare.yml` (that checks the *deployed* indexer against
  the *live* archive; this checks *code under test* against a *local* node).

## References

- Static e2e harness this extends: `tests/e2e/fidelity.sh`, `ops/fidelity-check.py`.
- Ledger authority proof: `docs/ledger-calculations.md`.
- Reconcile bug this would have caught: PR #119.
