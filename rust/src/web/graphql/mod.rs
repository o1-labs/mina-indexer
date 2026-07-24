//! GraphQL server & helpers

mod block;
mod date_time;
mod long;
mod pk;
mod timing;
mod txn;

pub mod accounts;
pub mod actions;
pub mod blocks;
pub mod charts;
pub mod events;
pub mod gen;
pub mod internal_commands;
pub mod snarks;
pub mod staged_ledgers;
pub mod stakes;
pub mod time_locks;
pub mod tokens;
pub mod verification_key;
pub mod top_snarkers;
pub mod top_stakers;
pub mod transactions;
pub mod version;

use super::ENDPOINT_GRAPHQL;
use crate::{
    base::state_hash::StateHash,
    block::{precomputed::PrecomputedBlock, store::BlockStore},
    constants::*,
    store::IndexerStore,
};
use actix_web::HttpResponse;
use anyhow::Context as aContext;
use async_graphql::{
    extensions::{Extension, ExtensionContext, ExtensionFactory, NextRequest},
    http::GraphiQLSource,
    Context, EmptyMutation, EmptySubscription, MergedObject, Response, Schema, ServerError,
};
use date_time::DateTime;
use long::Long;
use std::{sync::Arc, time::Duration};

#[derive(MergedObject, Default)]
pub struct Root(
    blocks::BlocksQueryRoot,
    actions::ActionsQueryRoot,
    events::EventsQueryRoot,
    stakes::StakesQueryRoot,
    accounts::AccountQueryRoot,
    transactions::TransactionsQueryRoot,
    internal_commands::InternalCommandQueryRoot,
    snarks::SnarkQueryRoot,
    staged_ledgers::StagedLedgerQueryRoot,
    tokens::TokensQueryRoot,
    top_stakers::TopStakersQueryRoot,
    top_snarkers::TopSnarkersQueryRoot,
    time_locks::TimeLocksQueryRoot,
    charts::ChartsQueryRoot,
    verification_key::VerificationKeyQueryRoot,
    version::VersionQueryRoot,
);

/// Extension that aborts a GraphQL request exceeding a wall-clock budget — a guard
/// against slow-but-valid queries that pass depth/complexity validation yet still tie
/// up a worker. Registered only when the configured timeout is non-zero.
struct Timeout {
    duration: Duration,
}

impl ExtensionFactory for Timeout {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(TimeoutExtension {
            duration: self.duration,
        })
    }
}

struct TimeoutExtension {
    duration: Duration,
}

#[async_graphql::async_trait::async_trait]
impl Extension for TimeoutExtension {
    async fn request(&self, ctx: &ExtensionContext<'_>, next: NextRequest<'_>) -> Response {
        match tokio::time::timeout(self.duration, next.run(ctx)).await {
            Ok(resp) => resp,
            Err(_) => Response::from_errors(vec![ServerError::new(
                format!(
                    "query exceeded the {}s execution timeout",
                    self.duration.as_secs()
                ),
                None,
            )]),
        }
    }
}

/// Build the GraphQL schema for all endpoints, applying the configured DoS guards:
/// query **depth** and **complexity** limits (rejected at *validation*, before any
/// resolver runs), an optional per-request execution **timeout**, and an optional
/// **introspection** switch. Each limit is operator-configurable via `ServerArgs`
/// (`--graphql-*` / `MINA_GRAPHQL_*`); a `0`/`false` value disables that guard.
///
/// NOTE: complexity here is *structural* (counts each field once) — it does not yet
/// multiply by list `limit:` sizes (that needs per-field complexity annotations, a
/// follow-up); list sizes are bounded today by the per-endpoint pagination caps.
pub fn build_schema(
    store: Arc<IndexerStore>,
    max_depth: usize,
    max_complexity: usize,
    timeout_secs: u64,
    disable_introspection: bool,
) -> Schema<Root, EmptyMutation, EmptySubscription> {
    let mut builder = Schema::build(Root::default(), EmptyMutation, EmptySubscription).data(store);
    if max_depth > 0 {
        builder = builder.limit_depth(max_depth);
    }
    if max_complexity > 0 {
        builder = builder.limit_complexity(max_complexity);
    }
    if timeout_secs > 0 {
        builder = builder.extension(Timeout {
            duration: Duration::from_secs(timeout_secs),
        });
    }
    if disable_introspection {
        builder = builder.disable_introspection();
    }
    builder.finish()
}

pub async fn indexer_graphiql() -> actix_web::Result<HttpResponse> {
    Ok(HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(GraphiQLSource::build().endpoint(ENDPOINT_GRAPHQL).finish()))
}

pub(crate) fn db<'a>(ctx: &'a Context) -> &'a Arc<IndexerStore> {
    ctx.data::<Arc<IndexerStore>>()
        .expect("Database should be in the context")
}

/// Convert epoch milliseconds to an ISO 8601 formatted [DateTime] Scalar.
pub(crate) fn date_time_to_scalar(millis: i64) -> DateTime {
    DateTime(millis_to_iso_date_string(millis))
}

/// Convenience function for obtaining a block's canonicity
pub(crate) fn get_block_canonicity(db: &Arc<IndexerStore>, state_hash: &StateHash) -> bool {
    use crate::canonicity::{store::CanonicityStore, Canonicity};
    db.get_block_canonicity(state_hash)
        .map(|status| matches!(status, Some(Canonicity::Canonical)))
        .unwrap_or(false)
}

pub(crate) fn get_block(db: &Arc<IndexerStore>, state_hash: &StateHash) -> PrecomputedBlock {
    db.get_block(state_hash)
        .with_context(|| format!("block missing from store {state_hash}"))
        .unwrap()
        .unwrap()
        .0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A `__type(name:"Query") { ofType { ofType { … name … } } }` introspection query
    /// nested `levels` deep — a schema-agnostic way to exercise the depth limiter
    /// (`__Type.ofType` is self-referential, and every field counts toward depth).
    fn nested_introspection(levels: usize) -> String {
        let mut q = String::from("{ __type(name: \"Query\") { ");
        for _ in 0..levels {
            q.push_str("ofType { ");
        }
        q.push_str("name ");
        for _ in 0..levels {
            q.push_str("} ");
        }
        q.push('}');
        q.push('}');
        q
    }

    #[tokio::test]
    async fn rejects_queries_deeper_than_the_limit() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let schema = build_schema(
            store,
            DEFAULT_GRAPHQL_MAX_DEPTH,
            DEFAULT_GRAPHQL_MAX_COMPLEXITY,
            DEFAULT_GRAPHQL_TIMEOUT_SECS,
            false,
        );

        // Well past the depth limit: rejected at validation, before any resolver runs.
        let deep = schema
            .execute(nested_introspection(DEFAULT_GRAPHQL_MAX_DEPTH + 5))
            .await;
        assert!(
            !deep.errors.is_empty(),
            "a query deeper than the depth limit must be rejected"
        );

        // A shallow query must still succeed — the guard doesn't affect real traffic.
        let shallow = schema.execute("{ __typename }").await;
        assert!(
            shallow.errors.is_empty(),
            "shallow query should pass, got: {:?}",
            shallow.errors
        );
    }

    #[tokio::test]
    async fn introspection_toggle_hides_the_schema() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let query = "{ __schema { queryType { name } } }";

        // Enabled (default): introspection resolves the schema.
        let on = build_schema(store.clone(), 0, 0, 0, false).execute(query).await;
        let on_data = serde_json::to_value(&on.data).unwrap();
        assert_ne!(
            on_data["__schema"],
            serde_json::Value::Null,
            "introspection enabled should resolve __schema, got: {on_data}"
        );

        // Disabled: the same query yields null — the schema is hidden.
        let off = build_schema(store, 0, 0, 0, true).execute(query).await;
        let off_data = serde_json::to_value(&off.data).unwrap();
        assert_eq!(
            off_data["__schema"],
            serde_json::Value::Null,
            "introspection disabled should hide __schema, got: {off_data}"
        );
    }

    /// Keeps the published GraphQL SDL (`docs/schema.graphql`) in lock-step with
    /// the schema, so clients can codegen against a committed contract. On drift
    /// this fails; regenerate with:
    ///
    /// ```text
    /// UPDATE_SCHEMA=1 cargo test --lib web::graphql::tests::published_sdl_is_current
    /// ```
    #[test]
    fn published_sdl_is_current() {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexerStore::new(dir.path(), true).unwrap());
        let sdl = build_schema(store, 0, 0, 0, false).sdl();

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/schema.graphql");
        if std::env::var("UPDATE_SCHEMA").is_ok() {
            std::fs::write(path, &sdl).expect("write docs/schema.graphql");
        } else {
            let on_disk = std::fs::read_to_string(path).unwrap_or_default();
            assert_eq!(
                sdl, on_disk,
                "GraphQL SDL drifted from docs/schema.graphql; regenerate with \
                 UPDATE_SCHEMA=1 cargo test --lib published_sdl_is_current"
            );
        }
    }
}
