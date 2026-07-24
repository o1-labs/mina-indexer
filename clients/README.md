# Mina Indexer clients

Official client libraries for the [Mina Indexer](../README.md) REST + GraphQL API.
Both lead with the **sync-aware health surface** (`/readyz`) so a caller can refuse
to trust an indexer that is still catching up to the chain tip.

| Client | Path | Package |
|---|---|---|
| Rust (async) | [`rust/`](./rust) | `mina-indexer-client` |
| TypeScript / JS | [`js/`](./js) | `@o1-labs/mina-indexer-client` |

Both expose the same shape:

- `healthz()` — liveness (process up + store answers)
- `readyz()` / `isReady()` — readiness (tip fresh enough to trust)
- `summary()` / `dbVersion()` — REST `/summary`
- `tipHeight()`, `accountsCount()` — typed convenience queries
- `graphql(query, vars)` — the escape hatch for any GraphQL query

## GraphQL schema

The full GraphQL SDL is published at [`../docs/schema.graphql`](../docs/schema.graphql)
and kept in lock-step with the server by a drift test (`published_sdl_is_current`).
Point your codegen at it. Regenerate after a schema change with:

```sh
UPDATE_SCHEMA=1 cargo test --lib web::graphql::tests::published_sdl_is_current
```

## Typical use

```ts
const client = new MinaIndexerClient("https://devnet-indexer.gcp.o1test.net");
if (!(await client.isReady())) return;         // don't trust a catching-up indexer
const height = await client.tipHeight();
```

See each client's README for install + full API.
