# CLAUDE.md

Guidance for Claude / coding agents working in this repository. Read this first; it
captures the architecture, the build/test loop, and the gotchas that aren't obvious
from a quick scan.

## What this is

The **Mina Indexer** builds and serves indices of the Mina blockchain from *precomputed
blocks* (the JSON blocks a Mina node logs). It ingests blocks into a [speedb][speedb]
(RocksDB fork) store, maintains a witness tree of recent chain history, computes
canonicity and ledgers, and serves **GraphQL** + **REST** on `:8080`. The Rust crate is
`mina_indexer`; the single binary is `mina-indexer`.

It is the source of truth behind [MinaSearch](https://minasearch.com).

## Build, test, run

The toolchain is pinned with **Nix** (`flake.nix` + `rust/rust-toolchain.toml`). Almost
everything runs inside `nix develop`.

```bash
# fast incremental build of the binary (what to use while iterating):
nix develop -c bash -c 'cd rust && GIT_COMMIT_HASH=dev cargo build --bin mina-indexer'

# rake is the task runner (see `rake -T`):
rake check         # clippy + format checks
rake test          # unit tests
rake dev           # quick regression battery
rake test_system   # the heavier system tests CI runs
rake build:oci_image

# reproducible Nix build of the binary / images (REQUIRES A CLEAN GIT TREE — the
# build reads `self.rev`; a dirty tree aborts or warns):
nix build .#mina-indexer
nix build .#dockerImage-mainnet     # or -devnet / -mesa
```

`GIT_COMMIT_HASH` is injected at build time (`flake.nix` sets it from the git rev; for
ad-hoc `cargo build` pass any value). Set `ulimit -n` ≥ 4096 before running the indexer.

## Repository layout

- `rust/src/` — the crate.
  - `bin/mina-indexer.rs` — CLI entry. Subcommands: `server {start,shutdown}`,
    `database {create,snapshot,restore,version}`, the flattened **client** query
    commands, and `version`.
  - `server.rs` — `IndexerConfiguration`, `start_indexer`, the fetch/reconcile timer
    loop, genesis/version constructors (`ChainId`, `GenesisVersion`, `IndexerVersion`).
  - `state/mod.rs` — `IndexerState`, the witness tree, `block_pipeline`.
  - `store/` — speedb wrapper (`IndexerStore`), snapshot/restore, column families.
  - `block/`, `ledger/`, `command/`, `canonicity/`, `chain/` — domain model.
  - `mina_blocks/v2/` — the V2 (hardfork/devnet/mesa) precomputed-block parser.
  - `web/` — actix-web server; `web/rest/` REST handlers, GraphQL alongside.
  - `cli/` — `DatabaseArgs` (shared db flags) and `ServerArgs` (server-start flags).
  - `constants.rs` — genesis hashes, chain ids, thresholds, defaults.
- `ops/` — operational tooling: block fetchers, per-network image entrypoints, Debian
  packaging, and a large pile of legacy Ruby/shell block-wrangling scripts.
- `docs/` — design deep-dives + the operator guide (`docs/operating.md`). Start at
  `docs/README.md`.
- `flake.nix` — toolchain, packages, and the per-network OCI image factory.
- `.github/workflows/` — `debian.yml` (deb build/publish) and `oci-image.yml` (the
  3-network image matrix). **There is no `cargo fmt` CI gate** — don't reflexively run
  `cargo fmt` across files; it produces large unrelated diffs. Format only what you touch.

## Key concepts (don't re-derive these)

- **Network is selected by genesis HASH, not `--network`.** `bin/mina-indexer.rs` (~L389)
  dispatches on `--genesis-hash`: `MESA_GENESIS_HASH` → V2/mesa, `DEVNET_GENESIS_HASH` →
  V2/devnet, `HARDFORK_GENESIS_HASH` → V2/v2, else → V1/mainnet. The `--network` string is
  mostly cosmetic (and the filename prefix the fetcher writes). Genesis hashes live in
  `constants.rs`.
- **Embedded vs supplied genesis ledger.** No `--genesis-ledger` ⇒ the embedded ledger for
  that version (mainnet V1 = `rust/data/genesis_ledgers/mainnet.json`). Hardfork networks
  (mesa, devnet) supply a **state-dump** ledger at runtime via `--genesis-ledger`; their
  genesis *block* is embedded (transactions emptied) and the `genesis_state_hash` is
  remapped to the checkpoint root so the whole chain shares one genesis for canonicity.
- **Block filename contract:** `<network>-<height>-<hash>.json`, split on the first dash,
  height parsed as `u32` (`block/mod.rs` `extract_network_height_hash`). Fetchers must
  produce this exact shape (mesa's bucket uses a different prefix, so `mesa-pull` rewrites).
- **Block sources** (public GCS, no auth): mainnet/devnet from `gs://mina_network_block_data`
  (already correctly named); mesa from `gs://mesa-mut-precomputed-blocks` (needs prefix
  rewrite). The fetcher runs **synchronously** in the timer branch — a large fetch window
  starves reconcile, so the window default is small (15).
- **Ledgers are keyed by token:** `tokens: HashMap<TokenAddress, TokenLedger>`, each keyed
  by `(TokenAddress, PublicKey)`. Genesis accounts must be partitioned by token, not
  flattened into the MINA ledger.

## Capabilities added in this fork (o1-labs)

1. **Per-network configless OCI images** — three turnkey images (mainnet/devnet/mesa-mut)
   that `docker run` with zero flags and self-initialize. Built by the `mkIndexerImage`
   factory in `flake.nix`; per-network entrypoints in `ops/entrypoints/`. See `ops/README.md`.
2. **Trustless verification** — `server start --verify-block-exe <exe>` gates every
   live-ingested block on an external verifier (`verify_block` in `server.rs`, fail-closed).
   The images bake a `verify-block` shim but leave it **dormant** (opt-in + sidecar). See
   `ops/mesa-mut/TRUSTLESS-DEMO.md`.
3. **Observability** — Prometheus `/metrics`, structured logging (`MINA_LOG_FORMAT=json`,
   `RUST_LOG`), and periodic speedb checkpoints (`MINA_CHECKPOINT_DIR`) with
   `--restore-from-checkpoint` recovery. See `ops/OBSERVABILITY.md` and `docs/operating.md`.

## CLI & environment (full reference: `docs/operating.md`)

- Server-start flags of note: `--genesis-hash`, `--genesis-ledger`, `--database-dir`,
  `--blocks-dir`, `--fetch-new-blocks-exe`/`--fetch-new-blocks-delay`,
  `--missing-block-recovery-exe`/`-delay`, `--verify-block-exe`,
  `--restore-from-checkpoint`/`--restore-force`, `--web-port`/`--web-hostname`,
  `--log-level`.
- Environment: `MINA_LOG_FORMAT=json`, `RUST_LOG=<env-filter>`, `MINA_CHECKPOINT_DIR`,
  `MINA_CHECKPOINT_INTERVAL_SECS` (default 3600), `GIT_COMMIT_HASH` (build-time).

## Conventions

- Keep diffs minimal and matched to surrounding style. No repo-wide `cargo fmt`.
- The local runtime dir (`/data` in images, `./data` locally) and `dist/` are gitignored —
  never commit them.
- Commit/push only when asked; branch first if on the default branch.

[speedb]: https://github.com/speedb-io/speedb
