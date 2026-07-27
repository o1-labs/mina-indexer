# Performance comparison — mina-indexer vs archive-node-api (Phase 3)

The pitch for the indexer is that it replaces the **archive node + daemon +
PostgreSQL** stack with one configless workload. Phase 3 measures that on two
axes: **latency/throughput** on identical queries, and **resource footprint**.

## Latency / throughput

Both surfaces serve the same archive-node-api-compatible GraphQL
(`blocks(query, limit, sortBy)`, `events`, `actions`), so the *same* query hits
each and the numbers compare directly. `compare-perf.py` fires N requests at a
fixed concurrency and reports p50/p95/p99/max + throughput per query, then the
indexer-vs-archive ratio.

```sh
ops/bench/compare-perf.py \
  --indexer https://devnet-indexer.gcp.o1test.net/graphql \
  --archive https://devnet-archive-node-api.gcp.o1test.net/ \
  --requests 500 --concurrency 20
```

Stdlib only (no install). Non-zero exit if either side errors.

**Where to run it.** Against the *public* endpoints the result is round-trip
bound (~50–70 req/s at low concurrency) and it's impolite to saturate shared
prod — good for a smoke/relative check, not a saturation curve. For a real
throughput ceiling, run it inside the cluster against the two Services (no
internet RTT), or point `--indexer` at a dedicated benchmark pod. The
lightnet e2e harness is the natural home for a controlled, repeatable run.

Queries are intentionally read-heavy `blocks` variants — the hot path for an
explorer/gateway. Extend `QUERIES` with more shared operations as needed (keep
them ones *both* schemas answer identically, or the comparison isn't fair — e.g.
`networkState` exists only on the archive-node-api, and the two `blocks` `query`
filters differ, so those are excluded).

## Resource footprint — the bigger lever

Throughput parity matters less than what it costs to serve. Per the devnet
deployment specs:

| | archive-node-api stack | mina-indexer |
|---|---|---|
| Components | mina-daemon + mina-archive + PostgreSQL (+ the API server) | one binary |
| Pods / processes | several | 1 |
| Datastore | PostgreSQL (managed volume, WAL, vacuum) | embedded speedb, in-process |
| Memory | daemon (GBs) + Postgres (GBs) | **~1.9 GiB, flat** — measured live: ~1986 MiB resident, +0 MiB under query load |
| CPU | daemon consensus + archive ingest + PG | ~1.5 cores ingest, up to ~4.4 under heavy query |
| Ops surface | daemon keys/peers, PG backups/migrations, archive schema | one PVC, one config-less image |

The indexer derives the same query surface from precomputed blocks in a single
process with a fraction of the moving parts — the operational win is the point,
the latency parity just clears the bar.

> Fill the memory/CPU rows with measured `kubectl top pod` numbers from both
> stacks on the same network to make the table concrete for a given deployment.
