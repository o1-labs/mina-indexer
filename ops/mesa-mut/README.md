# mesa-mut: run a local indexer / connect to precomputed blocks

Tooling to bootstrap and run a local Mina Indexer for the **mesa-mut** network.

mesa-mut is a hardfork at height **297734** (its genesis,
`3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT`). Its precomputed blocks
live in the public bucket `gs://mesa-mut-precomputed-blocks`
(`mina-mesa-mut-1-<height>-<state_hash>.json`); its genesis ledger is the
hardfork **state dump** under `gs://o1labs-gitops-infrastructure/mina-mesa-mut-1/`.

## Run a local instance

```bash
# build the binary first (needs the Nix toolchain):
#   nix develop --command bash -c 'cd rust && cargo build --release'

export INSTANCE=~/mesa-indexer        # where db + ledger + blocks live
ops/mesa-mut/run-local.sh setup        # download the genesis ledger (~79MB gz -> ~900MB)
ops/mesa-mut/run-local.sh fetch 200    # download 200 blocks from genesis
ops/mesa-mut/run-local.sh create       # build the indexer database
ops/mesa-mut/run-local.sh start        # serve GraphQL/REST on :8080 (watches blocks dir)
ops/mesa-mut/run-local.sh status       # chain summary
ops/mesa-mut/run-local.sh stop
```

The server runs with `--blocks-dir`, so re-running `fetch` while it's up makes
it auto-ingest the new blocks. Query it:

```bash
curl -s localhost:8080/summary | jq
curl -s -X POST localhost:8080/graphql -H 'content-type: application/json' \
  -d '{"query":"{ blocks(query:{blockHeight:297736},limit:1){ blockHeight stateHash transactions{ userCommands{ hash kind amount fee } } } }"}'
```

## Lower-level helpers

- `fetch-blocks.sh OUTPUT_DIR [START] [END]` — download mesa-mut blocks from the
  bucket and rename `mina-mesa-mut-1-<h>-<hash>.json` -> `mesa-<h>-<hash>.json`
  (single-token network prefix the indexer's filename parser requires).
- `mesa-mut-blocks` (binary, `rust/src/bin/`) — `report` / `serve` / `diag` over
  a directory of precomputed blocks, decoding them with the crate's V2 parser.

## How the network is wired

The genesis (fork block, ledger, constants) and the txn-v3 decoder support live
in the crate itself (selected by `--network mesa --genesis-hash 3NLp6dKN…`). See
the `feat(mesa)` commit. The genesis ledger is supplied at runtime via
`--genesis-ledger` (the state dump); the genesis block is embedded in the binary.
