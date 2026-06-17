# Mina Indexer — mesa-mut Benchmark

Network: **mesa-mut** · Tip at benchmark time: **300579** (`synced: true`)
Host: 16 cores, 62.5 GiB RAM · Image: `mina-indexer:mesa-prod`

## Database size

| Component | Size |
|-----------|------|
| speedb database (`/data/db`) | **30 GB** (3,319 SST files) |
| Precomputed blocks (`/data/blocks`) | 21 GB (4,171 blocks) |
| Genesis ledger | 895 MB |

Covers the full mesa chain from genesis (297735) to tip.

## CPU / RAM

| State | CPU | RAM |
|-------|-----|-----|
| Idle (between block fetches) | ~0.4–1% | **1.89 GiB** (2.95%) |
| Ingesting (~360 blocks/cycle) | ~40–140% (≈1.5 cores) | ~1.84 GiB |
| Under query load | sustained ~140%, peak ~444% (4.4 cores) | **1.88 GiB** |

**RAM stays steady at ~1.9 GiB** regardless of query load or on-disk DB size —
speedb keeps a bounded block-cache working set and memory-maps the rest. CPU is
bursty: near-zero when idle, a few cores during ingest and heavy read load, with
large headroom on a 16-core host.

## Query throughput

ApacheBench, concurrency 50.

| Query | Throughput | p50 | p95 | p99 |
|-------|-----------:|----:|----:|----:|
| `GET /health` | **4,782 req/s** | 9 ms | 21 ms | 25 ms |
| `transactions(limit: 20)` | **14,863 req/s** | — | — | 10 ms |
| GraphQL `blocks(limit: 1)` | **2,996 req/s** | 18 ms | 33 ms | 55 ms |
| `GET /summary` | **622 req/s** | 35 ms | 292 ms | 486 ms |
| GraphQL `blocks(limit: 50)` + creator | **546 req/s** | 17 ms | 419 ms | 833 ms |
| zkApp `events` (per account) | ~110 req/s | — | — | 32 ms |
| zkApp `actions` (per account) | ~126 req/s | — | — | 15 ms |

## Notes

- **Point lookups are fast and cheap** — transaction and single-block queries
  sustain thousands of req/s at sub-30 ms p99.
- **Aggregations cost more** — `/summary` and large multi-row joins drop to
  ~550–620 req/s with a longer tail as speedb reaches across cold SSTs.
- **Reads do not disturb ingestion** — the node stayed `synced: true` throughout;
  reads use a separate path from the single writer.
- To scale heavy/aggregate read traffic, front the writer with read replicas
  (the store's `read_only(primary, secondary)` mode + snapshot/restore seeding)
  to isolate query load from ingestion.
