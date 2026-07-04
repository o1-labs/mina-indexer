# GraphQL guardian (optional)

A drop-in [**graphql-protect**](https://github.com/ldebruijn/graphql-protect) proxy that
enforces one uniform GraphQL security policy — query depth, complexity (aliases/tokens/
batch), body size, POST-only — in front of the Mina **node** and/or the **indexer**.

## Is this required? No.

The indexer already enforces depth / complexity / execution-timeout / introspection
**in-process** (`mina-indexer server start --graphql-max-depth …`), so a self-hosted
indexer is protected with **zero external components**. This guardian is for:

- the OCaml **node** GraphQL (default `:3085`) — which **can't** be guarded in-process,
  so an edge proxy is the only option there; and/or
- putting **one policy at the edge** across every service (defense-in-depth; also covers
  direct-port access / anyone bypassing the app).

Think of it as: **in-house guards = secure by default; this guardian = uniform
cross-service policy + the node.**

## Prerequisite: the upstream schema (required)

graphql-protect loads the upstream GraphQL **SDL at startup and exits if it's missing**.
Export it from your upstream once into `./schema.graphql`:

```sh
# from the node (or indexer) GraphQL endpoint, while introspection is still enabled:
npx get-graphql-schema http://localhost:3085/graphql > schema.graphql
# (or: rover graph introspect http://localhost:3085/graphql > schema.graphql)
```

> If you plan to disable introspection at the edge, introspect **once** to capture the
> SDL, then lock it down. `schema.graphql` is `.gitignore`d — it's environment-specific.

## Configure & run

```sh
cp .env.example .env          # set MINA_NODE_GRAPHQL_PORT (default 3085), etc.
docker compose up -d          # guardian listens on :${GUARDIAN_PORT:-8080}
```

The upstream port is **configurable** via `.env` (`MINA_NODE_GRAPHQL_PORT`, default `3085`)
— it's substituted into `protect.yml` at startup (graphql-protect has no env-based config
override, so `docker-compose.yml` renders the template).

Then **point clients at the guardian** (`:8080`) instead of the upstream, and firewall the
upstream so it's only reachable through the guardian.

## What it enforces (`protect.yml.template`)

| Guard | Key | Default |
|---|---|---|
| Field-depth limit | `max_depth.field.max` | 15 |
| List-depth limit | `max_depth.list.max` | 15 |
| Alias count | `max_aliases.max` | 15 |
| Token count | `max_tokens.max` | 10000 |
| Batch size | `max_batch.max` | 5 |
| Request body size | `web.request_body_max_bytes` | ~100 KB |
| POST-only | `enforce_post.enabled` | true |
| Hide upstream error internals | `obfuscate_upstream_errors` | true |

Tune the numbers to your clients' real queries. Two schema-aware features
(`block_field_suggestions`, `persisted_operations` / trusted-document allow-listing) are
**off** by default — enable them once `schema.graphql` is the real upstream SDL.
Persisted-operation allow-listing is the strongest control for a known client set (only
pre-registered queries pass) and is the recommended end-state for a public platform.

## Fronting the indexer too

Run a second instance (copy this dir, set `MINA_NODE_GRAPHQL_PORT=8080` → the indexer, and a
different `GUARDIAN_PORT`), or add the indexer as another upstream. Because the indexer's
in-app guards remain the backstop, the edge policy and the app policy reinforce each other.

## Notes / limits

- graphql-protect's config is a static YAML read via `-f`; there is **no** env override for
  arbitrary keys — hence the template + `sed` render step for the one value (the upstream
  URL) that must be deployment-configurable.
- This proxy protects **availability/abuse**; it is not a trust boundary for data
  correctness. The trustless-serving story is separate (see the indexer's inclusion-proof
  work).
