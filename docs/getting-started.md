# Getting started — querying the Mina Indexer

The indexer serves **GraphQL** and **REST** (plus Prometheus `/metrics`) on one port
(`:8080`). This is the five-minute path: run one, make your first query, gate on
readiness, and reach for a typed client. For the full operator reference see
[operating.md](operating.md); for the deployment story see [../ops/README.md](../ops/README.md).

There's a public devnet instance you can hit right now:
**`https://devnet-indexer.gcp.o1test.net`**.

## 1. Run one (optional — or use the public endpoint)

The turnkey per-network images are configless — `docker run` with zero flags and they
self-initialize (bootstrap from the public block bucket, then follow the tip):

```bash
docker run -p 8080:8080 ghcr.io/o1-labs/mina-indexer:latest-devnet
```

(`latest-mainnet` / `latest-mesa-mut` for the other networks. See
[../ops/README.md](../ops/README.md) for volumes, checkpoints, and sizing.)

## 2. Is it ready?

The indexer keeps ingesting to follow the chain tip; **don't trust query results until
it's caught up.** Gate on readiness:

```bash
curl -s http://localhost:8080/readyz        # 200 = tip is fresh; 503 = still catching up
curl -s http://localhost:8080/healthz       # 200 = process/store alive (liveness)
```

`/readyz` returns `{ ready, status, tip_height, tip_age_seconds, max_lag_seconds }`.
`/summary` gives the full chain summary (heights, supply, counts, `dbVersion`).

## 3. First GraphQL query

Open **GraphiQL** in a browser at `/graphql`, or `curl`:

```bash
curl -s http://localhost:8080/graphql \
  -H 'content-type: application/json' \
  -d '{"query":"{ blocks(limit: 3, sortBy: BLOCKHEIGHT_DESC) { blockHeight stateHash } }"}'
```

A few things to know:

- **The schema** is published at [schema.graphql](schema.graphql) (kept in lock-step
  with the server by a drift test) — point your codegen at it, or explore live via
  GraphiQL's docs panel.
- **Every list query paginates the same way**: `limit` (≤ 1000) + `offset`, with a
  sibling `xxxCount(query)` for totals. See
  [features/graphql-pagination.md](features/graphql-pagination.md).
- **Results follow a live tip.** Pin a `blockHeight`/`state_hash` filter where a query
  supports it for a stable view, or use `stagedLedgerAccounts` for a point-in-time
  ledger. See [reorg-behavior.md](reorg-behavior.md) for what's provisional vs final.

## 4. REST endpoints

| Path | What |
|---|---|
| `GET /summary` | Chain summary (heights, supply, account/block/command counts, versions) |
| `GET /healthz` | Liveness — process + store up |
| `GET /readyz` | Readiness — 200 only when the tip is fresh |
| `GET /metrics` | Prometheus metrics (`mina_indexer_*`) |
| `POST /graphql` | GraphQL endpoint (GraphiQL on `GET`) |

## 5. Typed clients

Rather than hand-rolling HTTP, use a client — both lead with the readiness gate:

- **Rust** — [`clients/rust`](../clients/rust) (`mina-indexer-client`)
- **TypeScript / JS** — [`clients/js`](../clients/js) (`@o1-labs/mina-indexer-client`)

```ts
import { MinaIndexerClient } from "@o1-labs/mina-indexer-client";
const client = new MinaIndexerClient("https://devnet-indexer.gcp.o1test.net");
if (await client.isReady()) {
  console.log("tip", await client.tipHeight(), "accounts", await client.accountsCount());
}
```

## 6. Migrating from Blockberry / an archive node

The GraphQL surface is **archive-node-api compatible** and covers the Blockberry
(Minascan) surface. See
[features/blockberry-endpoint-coverage.md](features/blockberry-endpoint-coverage.md)
for the per-endpoint source map (what the indexer answers directly vs. what a gateway
composes), and [correctness-fidelity-blockberry.md](correctness-fidelity-blockberry.md)
for the correctness evidence (the indexer's ledger proven equal to the chain's own
records).

## Where next

- [operating.md](operating.md) — CLI + env reference, tuning, observability, recovery.
- [schema.graphql](schema.graphql) — the full GraphQL SDL.
- [../ops/README.md](../ops/README.md) — deploying the configless images.
