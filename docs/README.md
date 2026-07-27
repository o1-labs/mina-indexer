# Mina Indexer documentation

## Start here

- [getting-started.md](getting-started.md) — run one (or use the public devnet
  endpoint), gate on readiness, make your first GraphQL/REST query, and reach for a
  typed client. The five-minute path.

## Operating & deploying

- [operating.md](operating.md) — run, configure (CLI + env reference), observe, and
  recover an instance.
- [disaster-recovery.md](disaster-recovery.md) — DR runbook: RTO/RPO, off-host backups
  ([`ops/dr/`](../ops/dr/)), restore procedures, and the failover playbook.
- [../ops/README.md](../ops/README.md) — the turnkey per-network configless OCI images
  (mainnet / devnet / mesa-mut).
- [../ops/OBSERVABILITY.md](../ops/OBSERVABILITY.md) — Prometheus metrics, structured
  logging, and periodic checkpoints.
- [../ops/mesa-mut/TRUSTLESS-DEMO.md](../ops/mesa-mut/TRUSTLESS-DEMO.md) — the trustless
  (SNARK-proof-gated) ingestion demo and architecture.
- [../ops/mesa-mut/README.md](../ops/mesa-mut/README.md) — running a local mesa-mut indexer.

## Design & internals

- [development-principles-and-practices.md](development-principles-and-practices.md)
- [indexer_store.md](indexer_store.md) — the speedb store layout.
- [canonical-chain-discovery.md](canonical-chain-discovery.md)
- [reorg-behavior.md](reorg-behavior.md) — reorg handling, finality zones (canonical
  threshold vs `k`), and which query results are provisional vs final.
- [ledger-calculations.md](ledger-calculations.md)
- [lightnet-integration-test.md](lightnet-integration-test.md) — scoping for the
  live node → indexer integration test (reconcile/reorg pipeline coverage).
- [understanding-mainnet-transaction-fees.md](understanding-mainnet-transaction-fees.md)
- [user-commands.md](user-commands.md)
- [state/](state/) — the witness tree and how blocks are added to it.
- [features/](features/) — feature-specific notes.

## For coding agents

- [../CLAUDE.md](../CLAUDE.md) — architecture, build/test loop, and gotchas.
