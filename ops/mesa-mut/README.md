# mesa-mut precomputed-blocks app

Tooling to **connect to the precomputed blocks of the `mesa-mut` network** and
decode them with the Mina Indexer's own block types.

`mesa-mut` (internal name `hetzner-pre-mesa-1`) is a **preflight / preview**
network running the post-hardfork protocol, so its precomputed blocks are in
**PCB V2** format. Its public, no-auth precomputed-block bucket is
`gs://mesa-hf-precomputed-blocks`, with objects named
`hetzner-pre-mesa-1-<height>-<state_hash>.json`.

## 1. Fetch blocks

```bash
ops/mesa-mut/fetch-blocks.sh ./mesa-mut-blocks 200
```

Downloads up to 200 blocks and renames them to the indexer's filename
convention `mesa-<height>-<state_hash>.json`.

> **Why rename?** The indexer parses a block file name as
> `<network>-<height>-<hash>.json`, splitting on the **first** dash to get the
> network and requiring the next segment to be a `u32` height
> (`extract_network_height_hash`, `rust/src/block/mod.rs`). The bucket's
> multi-dash `hetzner-pre-mesa-1-...` prefix would make it read `"pre"` as the
> height and panic, so we collapse the prefix to the single token `mesa`.

## 2. Report

```bash
mina-indexer-target/release/mesa-mut-blocks report --blocks-dir ./mesa-mut-blocks
mina-indexer-target/release/mesa-mut-blocks report --blocks-dir ./mesa-mut-blocks --json
```

Decodes every block with `PrecomputedBlock::parse_file(.., PcbVersion::V2)` and
prints a per-block table plus aggregate stats (height range, command counts,
SNARK counts, genesis state hash).

## 3. Serve

```bash
mesa-mut-blocks serve --blocks-dir ./mesa-mut-blocks --port 8080
```

| Route | Description |
|-------|-------------|
| `GET /` | index + summary |
| `GET /blocks` | all decoded block summaries (JSON) |
| `GET /blocks/{height}` | summaries at a height |
| `GET /blocks/{height}/raw` | raw precomputed block JSON |

## Building

The app is a binary target in the `mina-indexer` crate, built by the normal
flow (it needs `clang` + `mold` from the Nix dev shell):

```bash
nix develop --command bash -c 'cd rust && cargo build --release --bin mesa-mut-blocks'
```

## Scope note — why this app, and not full indexer ingestion

The full `mina-indexer database create` pipeline bootstraps a network from a
hardcoded **genesis block** (`GenesisBlock::new_v2()`) and **genesis ledger**
keyed off the *mainnet* hardfork hash (`HARDFORK_GENESIS_HASH`). `mesa-mut` has
its own genesis (`3NKKivyyG1o3WerC5ivPoJNWFBCAkwkTJTHfkE2Q6t6EvGBX7j63`,
height 1, slot 0), whose genesis **block** file is not published in the bucket
(the lowest available block is height 2). Wiring that genesis in would require a
mesa genesis block + ledger and new genesis constants — a separate change.

Decoding and serving individual precomputed blocks only needs the V2 decoder,
which is exactly what this app exercises end-to-end against real `mesa-mut`
data.
