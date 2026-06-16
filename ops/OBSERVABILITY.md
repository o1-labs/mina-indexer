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

### 2. speedb checkpoints — **MEDIUM, for resilience**
Long-running images accumulate a multi-GB DB. An ungraceful kill (OOM/SIGKILL) means WAL
replay on restart (slow). Options:
- Periodic, online, consistent **speedb `Checkpoint`** (hard-link based, cheap) on a timer/flag,
  so restart resumes from a recent consistent point.
- At minimum, document/wire the existing `create-snapshot` CLI into a sidecar cron + restore-on-boot.

### 3. Log improvements — **LOW, targeted only**
Current logging is workable; just close the blind spots that cost us during debugging:
- Log fetcher results at INFO (`fetch: N new blocks in Ts`) — fetch exe stdout is currently swallowed.
- Log a one-line reconcile summary even when it ingests 0 (`reconcile: scanned N, ingested 0, tip H`).
- Optional structured (JSON) log output for aggregation (behind a flag).
- (Parser diagnostics already improved: `UserCommandData` now surfaces the real variant error.)

## Suggested scope for this PR
Start with **#1 (Prometheus `/metrics`)** — highest signal-to-effort, directly addresses the
failure modes we hit. #2 and #3 as follow-ups if desired.
