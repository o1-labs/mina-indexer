# Serving-layer DoS / security tests

Functional tests for the HTTP serving-layer guards. Run with:

```bash
rake test_security      # just these
rake test_system        # full system suite (includes `security`)
```

The harness (`test_security` in `tests/regression.bash`) boots the indexer with
the guards **enabled and tightened** so each one actually fires, then asserts
behavior. It uses **two instances**: the functional guards run with rate
limiting *off* (so hurl's rapid requests don't self-throttle), and rate limiting
is exercised on its own low-limit instance.

No block data is needed — a genesis-only DB is enough, because every guard here
fires at request validation / the CORS layer, before any resolver or data access.

## Coverage

| File / check | Guard | Flag under test | Asserted behavior |
|---|---|---|---|
| `cors.hurl` | CORS allow-list | `--web-cors-allowed-origins` | allowed origin → `Access-Control-Allow-Origin` echoed; disallowed → header absent (200, but a browser blocks the read) |
| `graphql_depth.hurl` | Query depth | `--graphql-max-depth` | over-depth query → `errors[0].message == "Query is nested too deep."` |
| `graphql_complexity.hurl` | Query complexity | `--graphql-max-complexity` | 25 aliased fields (> limit 20) → `"Query is too complex."` |
| `graphql_introspection.hurl` | Introspection toggle | `--graphql-disable-introspection` | `__schema` resolves to `null` (not exposed) |
| `body_size.hurl` | Max request body size | `--web-max-body-bytes` | GraphQL POST over the cap → HTTP 413 |
| rate-limit burst (in `test_security`) | Rate limiting | `--web-rate-limit-per-second` / `--web-rate-limit-burst` | rapid burst → first requests 200, then HTTP 429 |

Timing note: the rate-limit check is mildly timing-dependent. It's written with a
generous margin (8 rapid requests against a burst of 3), but if it ever proves
flaky as a per-PR gate, move `security` out of the default `test_names` in
`tests/regression-test.rb` and run it via `rake test_security` on a schedule.

## Note on max body size

`--web-max-body-bytes` is enforced by a `Content-Length` middleware
(`enforce_body_limit` in `web/mod.rs`), added because actix's `PayloadConfig`
alone does **not** bound the GraphQL POST body — async-graphql reads the body
itself. `body_size.hurl` exercises this (a padded GraphQL POST over the cap →
413). Requests sent without a `Content-Length` (chunked) bypass the middleware;
the reverse proxy is the backstop for those (see [`ops/reverse-proxy/`](../../../ops/reverse-proxy/README.md)).
