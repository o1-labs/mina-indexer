# mainnet-e2e — vendored block fixtures for the e2e fidelity gate

These 57 `.json` files are **real mainnet precomputed blocks** (the JSON blocks a
Mina daemon logs) for heights **359605–359624**, the first 20 heights after the
mainnet hardfork genesis. They are the input to the end-to-end ledger-fidelity
gate driven by [`tests/e2e/fidelity.sh`](../../../../tests/e2e/fidelity.sh)
(`rake test_e2e`, and the `e2e` CI job).

## Why record-and-replay (why vendor them at all)

The test is a **golden replay**: ingest a fixed, real chain slice and assert the
indexer's served balances against ground truth. The ground truth needs no
external oracle — every V2 block states the post-block balance of each account it
touched in `accounts_accessed`, so **the block set is its own oracle**. The gate
just replays these blocks and checks the indexer agrees.

Vendoring the slice (rather than fetching or regenerating it) is deliberate:

- **Hermetic & deterministic.** No network at test time; the same bytes every run.
  Fetching from GCS in CI would add a flaky external dependency; replaying a live
  node is non-deterministic (chain tip and fork timing move).
- **Self-contained.** Mainnet uses the hardfork genesis ledger **embedded in the
  binary**, so no multi-hundred-MB genesis file is needed — just these blocks.
- **Fast.** The whole gate ingests + verifies in seconds.

## Why this specific range

- **Small.** Early post-hardfork blocks are ~10–100 KB; activity ramps up and by
  ~359750 individual blocks balloon to ~9 MB each. 359605–359624 is 1.9 MB total —
  in line with the rest of `rust/tests/data`. Extending far past this bloats the
  repo fast.
- **Exercises the paths that had bugs.** The slice contains **forks** (multiple
  blocks per height → the indexer reorgs during ingest) and **post-genesis account
  creations**, so it drives the exact code fixed in #87 (unapply underflow), #88
  (zkApp creation fee) and #90 (post-genesis staged reconstruction). This gate
  would have caught all three.

## Coupling to the harness

`tests/e2e/fidelity.sh` hardcodes `SLICE_TOP=359624` (the top height) and a small
settled-height `MARGIN`. **If you change the vendored range, update those.**

## How to regenerate / extend

Blocks come from the public GCS bucket `mina_network_block_data` (no auth), named
exactly `mainnet-<height>-<hash>.json` — the form the indexer expects. To refresh
or widen the slice, fetch every block (forks included) in the range:

```bash
API="https://storage.googleapis.com/storage/v1/b/mina_network_block_data/o"
OBJ="https://storage.googleapis.com/mina_network_block_data"
for h in $(seq 359605 359624); do
  curl -fsS "${API}?prefix=mainnet-${h}-&fields=items(name)" \
    | grep -oE 'mainnet-[0-9]+-[^"]+\.json' \
    | while read -r n; do curl -fsS "${OBJ}/${n}" -o "$n"; done
done
```

Keep the range small (see above) and re-check `SLICE_TOP`/`MARGIN` in the harness.
