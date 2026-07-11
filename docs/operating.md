# Operating the Mina Indexer

How to run, configure, observe, and recover a Mina Indexer instance. For the design
internals see the other docs in this directory; for the turnkey container images see
[`ops/README.md`](../ops/README.md).

## Ways to run it

| Mode | Use it for | Pointer |
|------|------------|---------|
| **Configless OCI image** | Production / demos. `docker run` with zero flags. | [`ops/README.md`](../ops/README.md) |
| **`server start` directly** | Local dev, custom networks, custom block sources. | below |
| **Debian package** | systemd-managed host install. | [`ops/debian/`](../ops/debian/) + `mina-indexer.env` |

The fastest turnkey path:

```bash
docker run -d --name indexer -p 8080:8080 \
  -v indexer-mainnet:/data \
  ghcr.io/o1-labs/mina-indexer:<tag>-mainnet
```

A volume on `/data` is optional but recommended — the DB, fetched blocks, sockets, and
checkpoints live there and should survive restarts.

## Host requirements

Sizing is driven mostly by **disk**, not RAM. The figures below are grounded in the
mesa-mut benchmark ([`ops/mesa-mut/benchmark.md`](../ops/mesa-mut/benchmark.md), full
chain at tip 300579 on a 16-core / 62.5 GiB host).

| Tier | vCPU | RAM | Disk (mesa-mut) | Disk (mainnet) | Use |
|------|------|-----|-----------------|----------------|-----|
| **Minimum viable** | 2 | 4 GB | 80 GB SSD | 250 GB SSD | low query traffic, tolerant of slow sync |
| **Recommended** | 4 | 8 GB | 100 GB SSD | 400 GB SSD | normal prod, healthy ingest + query headroom |
| **Heavy read load** | 8 | 8–16 GB | 150 GB SSD | 500 GB+ SSD | aggregations/`/summary`; front with read replicas |

**RAM** stays a steady **~1.9 GiB regardless of on-disk DB size or query load** — speedb
keeps a bounded block-cache working set and memory-maps the rest. 4 GB is a safe floor;
extra RAM mainly buys OS page cache, which cuts the cold-SST tail latency on aggregate
queries. The working set does **not** grow as the DB reaches tens of GB.

**CPU** is bursty, not sustained: near-zero when idle, **~1.5 cores during ingest**, and
peaks of **~4.4 cores under heavy query load**. A single vCPU is a real bottleneck — the
fetcher runs *synchronously* in the timer loop, so on one core a fetch window stalls
reconcile and the tip lags, and any query traffic contends head-on with ingestion. Use
**≥ 2 vCPU** (4 recommended).

**Disk** is the variable that matters and must be **SSD/NVMe** — throughput is dominated
by random reads across SST files; spinning disk will not keep up. mesa-mut is ~52 GB
today (30 GB speedb + 21 GB blocks + ~0.9 GB genesis) and grows with the chain; **mainnet
is several times larger** — budget generously.

> **`ulimit -n ≥ 4096` is mandatory** on any tier. The indexer opens many SST files; a
> too-low file-descriptor limit will crash it. Set it before launch (systemd:
> `LimitNOFILE=`; Docker: `--ulimit nofile=4096`).

For heavy aggregate query traffic, keep a single writer and front it with **read
replicas** (the store's `read_only(primary, secondary)` mode + snapshot/restore seeding)
rather than scaling up one box — reads use a separate path and don't disturb ingestion.

## Running `server start` by hand

The indexer self-initializes: if `--database-dir` has no DB yet it builds one from
`--blocks-dir`; otherwise it opens and syncs. A typical live-following invocation:

```bash
mina-indexer --socket /data/mi.sock server start \
  --network mainnet \
  --genesis-hash 3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/block-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/block-pull --missing-block-recovery-delay 120 \
  --web-port 8080
```

> **Network is chosen by `--genesis-hash`, not `--network`.** The hash selects the block
> parser version and chain config (see [CLAUDE.md](../CLAUDE.md)). Pass the matching hash
> for the network you intend to index. Hardfork networks (mesa, devnet) also need a
> `--genesis-ledger` state dump; mainnet uses its embedded ledger.

## CLI flag reference (`server start`)

Shared database flags (`cli/database.rs`) plus server flags (`cli/server.rs`):

| Flag | Default | Meaning |
|------|---------|---------|
| `--genesis-hash <HASH>` | mainnet hash | Selects network/parser version. |
| `--genesis-ledger <FILE>` | embedded | State-dump ledger (required for mesa/devnet). |
| `--database-dir <DIR>` | `/var/lib/mina-indexer/database` | speedb data dir. |
| `--blocks-dir <DIR>` | — | Watched precomputed-block dir (ingest source). |
| `--staking-ledgers-dir <DIR>` | — | Optional staking-ledger dir. |
| `--fetch-new-blocks-exe <EXE>` | — | Fetcher run as `EXE <network> <height> <dir>` on a timer. |
| `--fetch-new-blocks-delay <SEC>` | 180 | Seconds between fetch attempts. |
| `--missing-block-recovery-exe <EXE>` | — | Same contract; backfills gaps below the tip. |
| `--missing-block-recovery-delay <SEC>` | — | Seconds between recovery attempts. |
| `--missing-block-recovery-batch <BOOL>` | — | Recover all missing heights per pass. |
| `--blocks-retention-length <N>` | — (keep all) | Bound `--blocks-dir` growth: delete ingested block files below `tip − N` each cycle. Floored at `k=290`. See [Bounding block-dir growth](#bounding-block-dir-growth). |
| `--verify-block-exe <EXE>` | — | Trustless gate: ingest a block only if `EXE <network> <file>` exits 0 (fail-closed). |
| `--restore-from-checkpoint <DIR>` | — | Seed an empty `--database-dir` from `<DIR>/latest` before opening. |
| `--restore-force` | false | With the above, overwrite a non-empty `--database-dir`. |
| `--web-hostname <HOST>` | `0.0.0.0` | REST/GraphQL bind host. |
| `--web-port <PORT>` | `8080` | REST/GraphQL port. |
| `--log-level <LEVEL>` | info | Crate log level when `RUST_LOG` is unset. |
| `--canonical-threshold`, `--prune-interval`, `--ledger-cadence`, … | see `cli/database.rs` | Tuning knobs. |

Other subcommands: `server shutdown`, `database create|snapshot|restore|version|verify-integrity`,
the client query commands (`mina-indexer <query> …` against the running socket), and `version`.

### Checking a database for corruption

`database verify-integrity --database-dir <DIR>` opens the store **read-only** (safe to run
against a live indexer) and reports silent corruption: a schema-version mismatch, a missing
best tip, and — by walking the canonical chain — any holes, canonical blocks missing from the
store, or broken parent linkage. It prints a summary (add `--json` for machine output) and
**exits non-zero** if any problem is found, so it drops into a cron/healthcheck:

```bash
mina-indexer database verify-integrity --database-dir /data/db || alert "indexer db integrity!"
```

## Environment variables

| Variable | Default | Effect |
|----------|---------|--------|
| `MINA_LOG_FORMAT` | human | `json` ⇒ one structured JSON log object per line (for Loki/ELK/Datadog). |
| `RUST_LOG` | unset | Standard `tracing`/`env_logger` filter; overrides `--log-level`. e.g. `warn,mina_indexer=debug`. |
| `MINA_CHECKPOINT_DIR` | unset | Enables periodic speedb checkpoints to `<dir>/latest`. |
| `MINA_CHECKPOINT_INTERVAL_SECS` | 3600 | Checkpoint cadence (hourly by default). |
| `MINA_BLOCKS_RETENTION_LENGTH` | 1000 (images) | Block-file retention window the configless image entrypoints pass to `--blocks-retention-length`. Set `0` to disable and keep every block. |
| `GIT_COMMIT_HASH` | — | Build-time version stamp (set by Nix). |

## Observability

- **Metrics:** `GET /metrics` — Prometheus exposition (blocks processed, ingest/fetch
  latency histograms, best-tip height, tip age, synced flag, dangling branches, reconcile
  counts, blocks pruned, ingest/fetch failure counters, HTTP request histogram, DB-size
  gauges). Point Prometheus at `:8080/metrics`.
- **Health / summary:** `GET /health`, `GET /summary` (chain summary as JSON).
- **Logs:** human-readable by default; set `MINA_LOG_FORMAT=json` for aggregation. Filter
  with `RUST_LOG`.

Ready-to-load **Prometheus alert rules + SLO recording rules** and an importable
**Grafana dashboard** are in [`ops/observability/`](../ops/observability/README.md), with
the full exported-metric reference. Design rationale and the metric list:
[`ops/OBSERVABILITY.md`](../ops/OBSERVABILITY.md).

## Checkpoints & recovery

> For the full disaster-recovery story — RTO/RPO, **off-host** scheduled backups
> ([`ops/dr/`](../ops/dr/)), restore procedures, and the failover playbook — see the
> [disaster-recovery runbook](disaster-recovery.md). Checkpoints below are the *on-host*
> (tight-RPO) layer; they do not survive losing the disk.

A speedb checkpoint dir is itself a complete, openable database. With `MINA_CHECKPOINT_DIR`
set, the indexer writes a consistent, hard-link-cheap checkpoint to `<dir>/latest` every
`MINA_CHECKPOINT_INTERVAL_SECS` (atomic tmp+rename, so `latest` is always complete).

**Recovery** is just making `latest` the active DB, via `--restore-from-checkpoint <dir>`:

- An **empty/absent** `--database-dir` is seeded from `<dir>/latest`, then opened normally.
- An **already-populated** `--database-dir` is opened as-is (it is usually newer than the
  checkpoint); pass `--restore-force` to discard it and restore from the checkpoint instead.
- Do **not** point `--database-dir` straight at `<dir>/latest` — the periodic writer
  replaces `latest` on each tick and would delete the running DB out from under itself.

The three configless images set `MINA_CHECKPOINT_DIR=/data/checkpoints` and pass
`--restore-from-checkpoint /data/checkpoints` (no force): hourly checkpoints by default, and
a wiped or fresh-but-checkpointed `/data` self-heals on boot while a healthy DB is never
clobbered. For a corrupt-but-present DB, clear `/data/db` (or run once with `--restore-force`).

## Bounding block-dir growth

`--blocks-dir` is an **ingest staging cache**, not a serving store. Once a block is parsed
into the speedb DB it is never read from disk again — queries serve from the DB, and the
only on-disk re-reader (`reconcile_blocks_dir`) looks no deeper than `tip − k` (`k = 290`,
the transition-frontier depth). So block files older than the frontier are dead weight, and
on a tip-following node `--blocks-dir` otherwise grows without bound.

`--blocks-retention-length <N>` caps it: on each fetch/reconcile cycle the indexer deletes
ingested block files below `tip − N`. It is **off by default** (every block is kept). When
set:

- The window is **floored at `k = 290`** — a smaller `N` is silently raised to `k`, so the
  blocks reconcile depends on are never deleted.
- Pruning is **safe and lossless for queries**: the data is already in the DB. The DB
  (`--database-dir`) is the durable index and is never touched by this.
- Deletions are counted by the `mina_indexer_blocks_pruned_total` metric and logged
  (`Pruned N … block file(s) below height H (… MiB freed)`).

The three configless images enable it by default at **1000 blocks**
(`MINA_BLOCKS_RETENTION_LENGTH=1000`); set `MINA_BLOCKS_RETENTION_LENGTH=0` to disable and
retain every fetched block. This keeps `/data/blocks` bounded to a small recent window
instead of the full chain (e.g. mesa-mut's full block set is ~21 GB).

> The older `database snapshot` / `database restore` subcommands are the **tarred** form of
> the same checkpoint (a single archive file + version stamp), for manual backup/transfer.
> The periodic checkpoint is the raw, untarred form, kept raw so hourly writes stay cheap.

## Trustless verification (opt-in)

`--verify-block-exe <exe>` makes the indexer gate every live-ingested block on an external
verifier: it runs `exe <network> <block-file>` and ingests only on exit 0 (unreachable
verifier or non-zero exit ⇒ block rejected, fail-closed). Paired with a SNARK-proof
verifier sidecar, the indexer trusts *math* rather than whoever served the block. The
configless images bake a `verify-block` shim but do **not** enable it by default. End-to-end
runbook: [`ops/mesa-mut/TRUSTLESS-DEMO.md`](../ops/mesa-mut/TRUSTLESS-DEMO.md).

## Public deployment (reverse proxy)

The indexer serves plain **HTTP on `:8080`** and has no built-in TLS or auth. For
any public/multi-tenant deployment, run it behind a reverse proxy that terminates
TLS and enforces authentication and edge rate limiting. Reference nginx and Caddy
configs (TLS + auth + rate-limit + body cap, with `/metrics` blocked) are in
[`ops/reverse-proxy/`](../ops/reverse-proxy/README.md).

Turn on the app-level guards too (they're defense-in-depth alongside the proxy):
`--web-cors-allowed-origins`, `--web-request-timeout-secs`, `--web-max-body-bytes`,
`--web-rate-limit-per-second` / `--web-rate-limit-burst`, and the `--graphql-*`
depth/complexity/timeout limits. See the CLI flag reference above.

## Querying

```bash
curl -s localhost:8080/summary | jq
curl -s localhost:8080/health

curl -s -X POST localhost:8080/graphql -H 'content-type: application/json' \
  -d '{"query":"{ blocks(query:{blockHeight:260846}, limit:1){ blockHeight stateHash } }"}'
```

GraphiQL is served at `http://localhost:8080/graphql` in a browser.
