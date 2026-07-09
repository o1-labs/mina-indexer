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
| rate-limit burst (in `test_security`) | Rate limiting | `--web-rate-limit-per-second` / `--web-rate-limit-burst` | rapid burst → first requests 200, then HTTP 429 |

Timing note: the rate-limit check is mildly timing-dependent. It's written with a
generous margin (8 rapid requests against a burst of 3), but if it ever proves
flaky as a per-PR gate, move `security` out of the default `test_names` in
`tests/regression-test.rb` and run it via `rake test_security` on a schedule.

## Known gap — max request body size (NOT yet enforced on GraphQL)

`--web-max-body-bytes` (#45) is applied via actix `PayloadConfig`, which the
async-graphql handler **does not honor** — it reads the request body itself. A
GraphQL POST larger than the configured cap currently returns **200, not 413**:

```bash
# with --web-max-body-bytes 1024
curl -s -o /dev/null -w '%{http_code}' -X POST localhost:8080/graphql \
  -H 'content-type: application/json' --data "$(python3 -c 'print("x"*2000)')"
# => 200   (expected: 413)
```

So there is intentionally **no `body_size.hurl` yet** — it would fail against
current `main`. The fix is a Content-Length-checking middleware (rejects
oversized bodies with 413 before the handler runs); once that lands, add:

```hurl
POST {{url}}/graphql
# body > --web-max-body-bytes
HTTP 413
```
