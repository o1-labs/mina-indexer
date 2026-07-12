# Observability assets — Prometheus alerts + Grafana dashboard

Ready-to-load monitoring for a running indexer. The indexer exposes Prometheus
metrics at `GET /metrics` on the web port (default `:8080`); point a scrape at
it, load the alert rules, and import the dashboard.

```
ops/observability/
├── prometheus/alerts.yml               # alerting + SLO recording rules
└── grafana/mina-indexer-dashboard.json # importable dashboard
```

See also `ops/OBSERVABILITY.md` (design/rationale) and `docs/operating.md`
(env vars, checkpoints, restore).

## 1. Scrape the indexer

```yaml
# prometheus.yml
scrape_configs:
  - job_name: mina-indexer          # alerts select on job="mina-indexer"
    static_configs:
      - targets: ["indexer-host:8080"]
```

## 2. Load the alert + SLO rules

```yaml
# prometheus.yml
rule_files:
  - /etc/prometheus/mina-indexer/alerts.yml
```

Copy `prometheus/alerts.yml` there and reload (`kill -HUP` / `POST /-/reload`).
A few thresholds are deployment-specific — grep for `TUNE:` and set them for
your network and disk budget before wiring to a pager. In particular:

- `IndexerDbSizeApproachingBudget` / `IndexerDbGrowthProjectedHigh` — set the
  `500e9` byte budget to ~80% of your DB volume (Prometheus can't see the
  volume's free space, so the alert is on the store's own footprint).
- `IndexerTipLagHigh`, `IndexerFetchSlow`, `IndexerHighQueryLatency` — tune to
  your freshness / latency SLOs and `--fetch-new-blocks-delay`.

## 3. Import the dashboard

Grafana → Dashboards → New → Import → upload
`grafana/mina-indexer-dashboard.json`, then pick your Prometheus datasource
when prompted. Rows: **Sync & tip**, **Ingest**, **Fetcher**, **HTTP serving**,
**Storage & capacity**.

## SLOs (what the alerts encode)

| SLO | Indicator | Alert |
|---|---|---|
| Synced ≥ 99.9% (30d) | `slo:mina_indexer_synced:ratio30d` | `IndexerNotSynced` (tip stale 10m) |
| Tip freshness | `mina_indexer_tip_age_seconds` | `IndexerTipLagHigh` |
| p99 query latency ≤ 2s | `slo:mina_indexer_http_latency:p99_5m` | `IndexerHighQueryLatency` |
| HTTP 5xx < 5% | `slo:mina_indexer_http_errors:ratio5m` | `IndexerHighErrorRate` |
| Ingest making progress | `blocks_processed_total`, `blocks_ingest_failed_total` | `IndexerIngestStalled`, `IndexerIngestFailures` |
| Fetcher healthy | `fetch_failures_total`, `fetch_seconds` | `IndexerFetchFailing`, `IndexerFetchSlow` |
| Disk headroom | `db_sst_files_bytes` | `IndexerDbSize*` |

## Exported metrics (reference)

All series are registered at startup, so they appear in `/metrics` before their
first event. Source: `rust/src/metrics.rs`.

### Sync / tip (gauges)
| Metric | Meaning |
|---|---|
| `mina_indexer_best_tip_height` | Blockchain length of the best tip |
| `mina_indexer_tip_age_seconds` | Seconds since the tip's block timestamp |
| `mina_indexer_synced` | 1 if the tip is within ~2 slots of now, else 0 |
| `mina_indexer_dangling_branches` | Disconnected branches in the witness tree |

### Ingest
| Metric | Type | Meaning |
|---|---|---|
| `mina_indexer_blocks_processed_total` | counter | Blocks applied to the witness tree |
| `mina_indexer_reconcile_ingested_total` | counter | Blocks ingested by the reconcile safety net |
| `mina_indexer_blocks_ingest_failed_total` | counter | Blocks that parsed but failed to apply |
| `mina_indexer_blocks_pruned_total` | counter | Ingested block files deleted by retention pruning |
| `mina_indexer_block_ingest_seconds` | histogram | Per-block witness-tree apply time |

### Fetcher
| Metric | Type | Meaning |
|---|---|---|
| `mina_indexer_fetch_invocations_total` | counter | fetch-new-blocks invocations |
| `mina_indexer_fetch_failures_total` | counter | fetch-new-blocks invocations that failed to spawn |
| `mina_indexer_fetch_seconds` | histogram | fetch-new-blocks wall time |

### HTTP serving
| Metric | Type | Meaning |
|---|---|---|
| `mina_indexer_http_request_duration_seconds` | histogram | Request latency, labelled `endpoint` (route pattern), `method`, `status`. Its `_count` is the request/error counter. |

### Storage (gauges, refreshed at scrape time)
| Metric | Meaning |
|---|---|
| `mina_indexer_db_sst_files_bytes` | On-disk SST footprint (the real disk usage) |
| `mina_indexer_db_estimated_live_data_bytes` | Estimated logical live data size |
| `mina_indexer_db_estimated_num_keys` | Estimated total key count |

### Resources (gauges, refreshed at scrape time)
| Metric | Meaning |
|---|---|
| `mina_indexer_process_resident_memory_bytes` | Process RSS (from `/proc/self/status`) |
| `mina_indexer_process_open_fds` | Open file descriptors (watch vs the `ulimit -n` 4096 floor) |
| `mina_indexer_db_block_cache_bytes` | speedb block-cache RAM (read working set) |
| `mina_indexer_db_table_readers_bytes` | RAM held by SST index/filter blocks |
| `mina_indexer_db_running_compactions` | Compactions currently running |
| `mina_indexer_db_compaction_pending` | 1 if a compaction is pending, else 0 |
