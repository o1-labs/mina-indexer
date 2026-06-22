# Trustless (verifiable) indexer responses

**Status:** design / roadmap. **Scope:** tiers 0–2 (blocks/txs for all history; account balances for
the finality window). Historical account proofs (epoch checkpoints / per-block) are explicitly **out
of scope** here — see "Not in scope".

## Goal

Let a client query a **remote** indexer it does not operate and **verify every response
cryptographically**, so it trusts the *math*, not the indexer operator. This is the answer to "a CEX
won't host the indexer": one shared indexer can serve many parties, each of whom trusts no one.

The indexer is **untrusted**. Every guarantee comes from the client re-checking a proof. A malicious
or buggy indexer can return wrong data, but the client rejects it. The indexer's only job is to
*attach the proof material* alongside the data it already serves.

## What already exists (so this is mostly plumbing)

- The indexer **only ingests proof-verified blocks** — it already runs the recursive-SNARK check on
  ingest, and it stores the precomputed blocks (with `protocol_state_proof`) under `blocks-dir`.
- The verification primitives live in `mina-verify` (the light node's trust gate):
  `verify_block`, `verify_account_inclusion`, `ledger_root`, `MerklePath`. The client re-runs these;
  the indexer just needs to hand over the inputs.
- The light node already produces Merkle-inclusion proofs for **current** account state
  (`verify_account_at_root`), via a ledger sync. Tier 2 reuses that machinery for the finalized root.

## The proof envelope

Add an **optional** proof block to existing responses (off by default; requested via a flag/param,
e.g. `?proof=1` or an Accept header). When present:

```jsonc
{
  "data": { /* the existing response */ },
  "proof": {
    // Tier 1 — block/tx authenticity (all history):
    "block_binprot": "<hex>",          // the full block incl. protocol_state_proof, as the client parses it
    "canonicity": {                    // proof the block is canonical, not a valid-but-orphaned fork
      "anchor_state_hash": "3N…",      // a tip the client independently trusts (from its light node)
      "parent_chain": ["3N…", "3N…"]   // state-hash chain anchor → … → target (each link = parent_hash)
    },                                 //   …or omit parent_chain and rely on target being ≥ k deep
    // Tier 2 — account state (finality window only):
    "account": {
      "leaf": { /* mina account */ },
      "merkle_path": [ /* MerklePath: left/right siblings, root→leaf */ ],
      "ledger_root_block": "3N…"       // the block whose blockchain_state carries this ledger root
    }
  }
}
```

The envelope is deliberately *self-contained per response*: the client needs nothing from the indexer
it hasn't verified.

## Work, by endpoint

### Tier 1 — `/block`, transaction search (ALL history)
- Serve `block_binprot` from the stored precomputed block. **Retention:** ensure `blocks-dir` /
  block store keeps the full block incl. the proof for the heights served (today's
  `--blocks-retention-length` prunes old block *files* once they're in speedb — confirm the proof
  survives, or persist it).
- Emit the **canonicity** material from the indexer's own canonical-chain index
  (`canonicity_store`): the parent-hash chain from a recent canonical tip down to the target, or just
  the target's depth so the client can apply k-finality. This is the only genuinely new logic.
- Tx inclusion is free: a verified block contains its `staged_ledger_diff`; the client checks the tx
  is in it.

### Tier 2 — `/account` at a finalized height (≈ last `k`=290 blocks ≈ ~½ day)
- Generate `merkle_path` for the account against a **finalized ledger root** — i.e. the frontier/
  snarked root the light node already syncs. Reuse the light-node ledger-sync + path generation; the
  indexer exposes the path plus `ledger_root_block` (the block whose `blockchain_state` commits that
  root).
- Anchor: the same canonicity material as Tier 1 for `ledger_root_block`.

## Not in scope (tiers 3–4)
- Account proofs **older than the finality window** (a balance "as of last month"). That needs
  retained historical ledger Merkle trees (epoch-checkpoint snapshots, or a versioned store). Big
  storage + the heavy audit live there. Add later, behind a separate retention policy.

## Security notes (small but sharp)
- The indexer is untrusted; do **not** add server-side "trust me" shortcuts. The proof must let the
  client reach the answer with only `mina-verify` + a genesis anchor + its own light-node tip.
- The **canonicity anchor is the #1 correctness risk** — a valid-but-orphaned block must not be
  presentable as "the block at height H". Keep the parent-chain/k-depth logic minimal and reviewed.
- Pair with `MinaMesh/docs/trustless-responses.md` — the consuming (verifying) side.
