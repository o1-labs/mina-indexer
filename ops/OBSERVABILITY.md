# Indexer observability — assessment & proposal

Context: the configless per-network images run the indexer unattended, fetching and
ingesting live blocks. While bringing devnet up we hit two failure modes that were
**invisible without reading logs/disk by hand** — a fetcher that silently blocked the
ingest loop, and an O(tree) ingest slowdown. That's the lens for this assessment.

## Current state
| Area | Today |
|---|---|
| Metrics | **none** — no `/metrics`, no Prometheus |
| Logging | `stderrlog` (unstructured, `RUST_LOG` levels) + INFO progress lines |
| Health | `/health` REST (status, `synced`, `tip_height`, `tip_age_seconds`) — solid liveness/readiness probe |
| DB resilience | manual `mina-indexer ... create-snapshot` CLI + WAL flush on clean shutdown; no periodic checkpoint |

## Recommendation (priority order)

### 1. Prometheus `/metrics` endpoint — **HIGH value, do this**
The single biggest gap. Both devnet bring-up failures would have been a glance at a
graph. Expose on the existing actix server (port 8080). Proposed series:

- `mina_indexer_best_tip_height` (gauge) — ingest progress (would've shown the stall)
- `mina_indexer_tip_age_seconds`, `mina_indexer_synced` (gauges) — staleness (mirror `/health`)
- `mina_indexer_blocks_processed_total` (counter)
- `mina_indexer_block_ingest_duration_seconds` (histogram) — per-block apply time (would've shown the O(tree) slowdown)
- `mina_indexer_fetch_blocks_total{network,result}` + `mina_indexer_fetch_duration_seconds` (histogram) — fetcher health (would've shown the window-200 block)
- `mina_indexer_reconcile_ingested_total` (counter)
- `mina_indexer_witness_tree_dangling_branches`, `mina_indexer_db_size_bytes` (gauges)
- `mina_indexer_graphql_requests_total{op}` + latency histogram

Implementation: `prometheus` crate registry; instrument `block_pipeline`, the fetch/reconcile
timer branch, and the web layer; additive and low-risk. The internal `blocks_processed` /
`bytes_processed` / `best_tip_block()` / `dangling_branches` already exist to feed it.

### 2. speedb checkpoints — **DONE (opt-in)**
Long-running images accumulate a multi-GB DB. An ungraceful kill (OOM/SIGKILL) means WAL
replay on restart (slow). Implemented as a periodic, online, consistent **speedb `Checkpoint`**
(hard-link based, cheap) written atomically to `<dir>/latest`:
- Opt-in via `MINA_CHECKPOINT_DIR` (the three configless images set it to `/data/checkpoints`).
- Cadence via `MINA_CHECKPOINT_INTERVAL_SECS` (default `3600` = hourly).
- Runs on a tokio timer off a `spawn_blocking` worker (`spawn_periodic_checkpoints` in `server.rs`),
  so restart can resume from a recent consistent point instead of replaying a large WAL.

**Recovery.** A checkpoint dir is itself a complete, openable speedb DB, so recovery is just making
`<dir>/latest` the active DB — exposed as `server start --restore-from-checkpoint <dir>`:
- Seeds an **empty/absent** `--database-dir` from `<dir>/latest`, then opens it normally (Sync mode).
- An **already-populated** `--database-dir` is opened as-is (it is usually newer than the checkpoint);
  pass `--restore-force` to discard it and restore from the checkpoint instead.
- The three images bake `--restore-from-checkpoint /data/checkpoints` (no `--force`), so a wiped or
  fresh-but-checkpointed `/data` self-heals while a healthy DB is never clobbered. For a corrupt-but-present
  DB, clear `/data/db` (or run once with `--restore-force`) to force the restore.
- Do **not** point `--database-dir` straight at `<dir>/latest`: the periodic writer replaces `latest`
  on each tick and would delete the DB out from under the running process.

### 3. Log improvements — **LOW, targeted only**
Current logging is workable; just close the blind spots that cost us during debugging:
- Log fetcher results at INFO (`fetch: N new blocks in Ts`) — fetch exe stdout is currently swallowed.
- Log a one-line reconcile summary even when it ingests 0 (`reconcile: scanned N, ingested 0, tip H`).
- Optional structured (JSON) log output for aggregation (behind a flag).
- (Parser diagnostics already improved: `UserCommandData` now surfaces the real variant error.)

## Delivery (stacked PR train)
- **#1 Prometheus `/metrics`** — done (PR #12).
- **#3 structured JSON logging** — done (PR #13): `MINA_LOG_FORMAT=json`, `RUST_LOG`-overridable.
- **#2 periodic speedb checkpoints** — done (this PR): `MINA_CHECKPOINT_DIR` / `MINA_CHECKPOINT_INTERVAL_SECS`.
- **#4 block-dir retention** — done: `--blocks-retention-length` / `MINA_BLOCKS_RETENTION_LENGTH` bounds `blocks-dir` growth; deletions counted by `mina_indexer_blocks_pruned_total`. See [`docs/operating.md`](../docs/operating.md#bounding-block-dir-growth).
- **#5 dashboards + alerts** — done: ready-to-load Prometheus alert/SLO rules and an importable Grafana dashboard consuming the full metric surface (query histogram, ingest/fetch failure counters, DB-size gauges). See [`ops/observability/`](observability/README.md).
