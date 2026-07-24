# mina-indexer-client (Rust)

Async Rust client for the [Mina Indexer](https://github.com/o1-labs/mina-indexer)
REST + GraphQL API, leading with **sync-aware health checks** so callers can
refuse to trust an indexer that is still catching up.

Requires a stable Rust toolchain (≥ 1.86 — a transitive dependency's MSRV). The
main indexer crate pins an older toolchain via Nix; this client is standalone and
builds on system stable.

## Install

```toml
[dependencies]
mina-indexer-client = { git = "https://github.com/o1-labs/mina-indexer", package = "mina-indexer-client" }
tokio = { version = "1", features = ["full"] }
```

## Use

```rust
use mina_indexer_client::MinaIndexerClient;

#[tokio::main]
async fn main() -> Result<(), mina_indexer_client::Error> {
    let client = MinaIndexerClient::new("https://devnet-indexer.gcp.o1test.net");

    // Liveness / readiness
    assert!(client.healthz().await?);            // process up + store answers
    if !client.is_ready().await? {               // tip fresh enough to trust?
        let r = client.readyz().await?;
        eprintln!("catching up: {:?} ({}s behind budget {}s)",
            r.status, r.tip_age_seconds.unwrap_or(0), r.max_lag_seconds.unwrap_or(0));
        return Ok(());
    }

    // Queries
    println!("db version : {}", client.db_version().await?);
    println!("tip height : {}", client.tip_height().await?);
    println!("accounts   : {}", client.accounts_count(None).await?);

    // Anything else, via raw GraphQL:
    let data: serde_json::Value = client
        .graphql("{ timeLocks(bucket: YEAR) { date locked_supply } }", serde_json::json!({}))
        .await?;
    println!("{data}");
    Ok(())
}
```

## API

| Method | Endpoint | Returns |
|---|---|---|
| `healthz()` | `GET /healthz` | `bool` — liveness |
| `readyz()` | `GET /readyz` | `Readiness { ready, status, tip_height, tip_age_seconds, max_lag_seconds }` |
| `is_ready()` | `GET /readyz` | `bool` — gate queries on this |
| `summary()` | `GET /summary` | `serde_json::Value` |
| `db_version()` | `GET /summary` | `String` (e.g. `0.19.0-<git>`) |
| `tip_height()` | GraphQL | `u32` |
| `accounts_count(query)` | GraphQL | `u32` |
| `graphql::<T>(query, vars)` | `POST /graphql` | `T` — any query, typed by the caller |

`graphql` is the escape hatch: pass any query and deserialize `data` into your own
type. The typed helpers are thin wrappers over it.
