# Reference reverse-proxy config

The mina-indexer serves plain **HTTP on `:8080`** and is meant to run behind a
reverse proxy for any public/multi-tenant deployment. The proxy owns the things
an application server shouldn't: **TLS termination, authentication, and edge
rate limiting** keyed on the real client IP.

This directory ships two starting points — use one:

| File | When |
|------|------|
| [`Caddyfile`](./Caddyfile) | Lowest effort — Caddy auto-provisions & renews TLS. |
| [`nginx.conf`](./nginx.conf) | You already run nginx / want fine-grained control. |

Both are **references, not drop-in production configs** — change the hostname,
wire up real certs/auth, and review every limit against your traffic.

## What the proxy adds vs. the indexer's own guards

The indexer has app-level guards you should also turn on (see
`docs/operating.md` → *CLI flag reference*); the proxy is defense-in-depth and
covers what the app can't:

| Concern | Indexer flag (app-level) | Proxy (edge) |
|---------|--------------------------|--------------|
| TLS | — (HTTP only) | **TLS termination** |
| Auth | — | **Basic / bearer / auth-subrequest** |
| Rate limit | `--web-rate-limit-per-second` / `--web-rate-limit-burst` (per **peer** IP) | keyed on the **real client IP** (`X-Forwarded-For`) |
| Max body | `--web-max-body-bytes` | `client_max_body_size` / `request_body max_size` |
| Header/slowloris timeout | `--web-request-timeout-secs` | `client_header_timeout`, etc. |
| CORS | `--web-cors-allowed-origins` | (leave to the app) |
| GraphQL depth/complexity/timeout | `--graphql-*` | (leave to the app) |

**Why both rate limiters?** The indexer's limiter keys on the *peer* IP — which,
behind a proxy, is the proxy itself. Keep the edge limiter (real client IP) as
the primary control; the app-level knob is a backstop for when the indexer is
reached directly.

## Notes

- **Never expose `/metrics` publicly** — both configs block it. Scrape Prometheus
  over the private network instead.
- Put the proxy and indexer on the same host or a private network; don't bind
  the indexer to a public interface (`--web-hostname 127.0.0.1` if the proxy is
  local).
- For real multi-tenant auth (per-key quotas, revocation), front the proxy with
  an API gateway or an `auth_request` subrequest to your own service rather than
  a single static token.
