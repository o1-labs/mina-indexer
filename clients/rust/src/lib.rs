//! Async Rust client for the [Mina Indexer](https://github.com/o1-labs/mina-indexer)
//! REST + GraphQL API.
//!
//! The indexer serves REST (`/summary`, `/healthz`, `/readyz`) and GraphQL
//! (`/graphql`) on one port (`:8080` by default). This client wraps both, and
//! leads with the **health / sync** surface so a caller can refuse to trust an
//! indexer that is still catching up.
//!
//! ```no_run
//! # async fn run() -> Result<(), mina_indexer_client::Error> {
//! use mina_indexer_client::MinaIndexerClient;
//!
//! let client = MinaIndexerClient::new("https://devnet-indexer.gcp.o1test.net");
//!
//! // Gate on readiness before trusting query results.
//! if !client.is_ready().await? {
//!     eprintln!("indexer is catching up — not querying");
//!     return Ok(());
//! }
//!
//! let height = client.tip_height().await?;
//! let accounts = client.accounts_count(None).await?;
//! println!("tip {height}, {accounts} accounts");
//! # Ok(())
//! # }
//! ```

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Errors returned by the client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("graphql errors: {0}")]
    GraphQl(String),

    #[error("unexpected response shape: {0}")]
    Shape(String),
}

/// A client bound to one indexer base URL.
#[derive(Clone, Debug)]
pub struct MinaIndexerClient {
    base_url: String,
    http: reqwest::Client,
}

/// `/readyz` response — whether the indexer's tip is fresh enough to trust.
#[derive(Debug, Clone, Deserialize)]
pub struct Readiness {
    /// `true` when the best tip is within the indexer's lag budget.
    #[serde(default)]
    pub ready: bool,
    /// `ready`, `catching_up`, `bootstrapping`, or `store_unavailable`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tip_height: Option<u32>,
    #[serde(default)]
    pub tip_age_seconds: Option<u64>,
    #[serde(default)]
    pub max_lag_seconds: Option<u64>,
}

impl MinaIndexerClient {
    /// Create a client for `base_url` (e.g. `http://localhost:8080`). A trailing
    /// slash is trimmed.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self::with_client(base_url, reqwest::Client::new())
    }

    /// Create a client with a caller-provided [`reqwest::Client`] (custom
    /// timeouts, proxy, TLS, …).
    pub fn with_client(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self { base_url, http }
    }

    // ---- health / sync ----

    /// Liveness (`GET /healthz`): `true` if the process is up and the store
    /// answers. Does **not** consider sync state.
    pub async fn healthz(&self) -> Result<bool, Error> {
        let resp = self.http.get(self.url("/healthz")).send().await?;
        Ok(resp.status().is_success())
    }

    /// Readiness (`GET /readyz`): the full status object. `status_ok` on the
    /// HTTP response mirrors `Readiness::ready` (503 when not ready).
    pub async fn readyz(&self) -> Result<Readiness, Error> {
        let resp = self.http.get(self.url("/readyz")).send().await?;
        // /readyz returns 503 (with a body) when not ready — read the body
        // either way rather than erroring on the status.
        let readiness: Readiness = resp.json().await?;
        Ok(readiness)
    }

    /// Convenience: `true` only when the indexer reports itself ready (tip
    /// fresh). Gate queries on this.
    pub async fn is_ready(&self) -> Result<bool, Error> {
        Ok(self.readyz().await?.ready)
    }

    // ---- REST ----

    /// `GET /summary` — the blockchain summary object (raw JSON; field set is
    /// large and versioned, so it is returned untyped).
    pub async fn summary(&self) -> Result<serde_json::Value, Error> {
        let resp = self.http.get(self.url("/summary")).send().await?;
        Ok(resp.error_for_status()?.json().await?)
    }

    /// The indexer's store schema version, e.g. `0.19.0-<git>` (from `/summary`).
    pub async fn db_version(&self) -> Result<String, Error> {
        let summary = self.summary().await?;
        summary
            .get("dbVersion")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| Error::Shape("summary missing dbVersion".into()))
    }

    // ---- GraphQL ----

    /// Run a GraphQL query and deserialize the `data` field into `T`. Returns
    /// [`Error::GraphQl`] if the response carries `errors`.
    pub async fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<T, Error> {
        #[derive(Serialize)]
        struct Req<'a> {
            query: &'a str,
            variables: serde_json::Value,
        }
        #[derive(Deserialize)]
        struct Resp<T> {
            data: Option<T>,
            errors: Option<serde_json::Value>,
        }

        let resp: Resp<T> = self
            .http
            .post(self.url("/graphql"))
            .json(&Req { query, variables })
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        if let Some(errors) = resp.errors {
            return Err(Error::GraphQl(errors.to_string()));
        }
        resp.data
            .ok_or_else(|| Error::Shape("graphql response had no data".into()))
    }

    // ---- typed convenience queries ----

    /// Height of the canonical best tip.
    pub async fn tip_height(&self) -> Result<u32, Error> {
        #[derive(Deserialize)]
        struct Block {
            #[serde(rename = "blockHeight")]
            block_height: u32,
        }
        #[derive(Deserialize)]
        struct Data {
            blocks: Vec<Block>,
        }
        let data: Data = self
            .graphql(
                "{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true}) { blockHeight } }",
                serde_json::json!({}),
            )
            .await?;
        data.blocks
            .into_iter()
            .next()
            .map(|b| b.block_height)
            .ok_or_else(|| Error::Shape("no best block".into()))
    }

    /// Total number of accounts matching `query` (the whole ledger when `None`).
    /// Pass a GraphQL `AccountQueryInput` literal, e.g. `"{ balance_gte: 0 }"`.
    pub async fn accounts_count(&self, query: Option<&str>) -> Result<u32, Error> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "accountsCount")]
            accounts_count: u32,
        }
        let q = match query {
            Some(q) => format!("{{ accountsCount(query: {q}) }}"),
            None => "{ accountsCount }".to_string(),
        };
        let data: Data = self.graphql(&q, serde_json::json!({})).await?;
        Ok(data.accounts_count)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_trailing_slash() {
        let c = MinaIndexerClient::new("http://localhost:8080/");
        assert_eq!(c.url("/readyz"), "http://localhost:8080/readyz");
    }

    #[test]
    fn readiness_deserializes_partial() {
        // /readyz body when catching up
        let r: Readiness = serde_json::from_value(serde_json::json!({
            "status": "catching_up", "ready": false,
            "tip_height": 528207, "tip_age_seconds": 3_400_000, "max_lag_seconds": 600
        }))
        .unwrap();
        assert!(!r.ready);
        assert_eq!(r.status, "catching_up");
        assert_eq!(r.tip_height, Some(528207));
    }
}
