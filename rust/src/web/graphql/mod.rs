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
pub mod events;
pub mod gen;
pub mod internal_commands;
pub mod snarks;
pub mod staged_ledgers;
pub mod stakes;
pub mod tokens;
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
    http::GraphiQLSource, Context, EmptyMutation, EmptySubscription, MergedObject, Schema,
};
use date_time::DateTime;
use long::Long;
use std::sync::Arc;

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
    version::VersionQueryRoot,
);

/// Build the GraphQL schema for all endpoints, bounding query **depth** and
/// **complexity** as a DoS guard: a deeply-nested selection (the types are
/// self-referential, e.g. block → transactions → block → …) or a very large single
/// query is rejected at *validation*, before any resolver runs. A limit of `0`
/// disables that guard.
///
/// The limits are operator-configurable — see `ServerArgs::graphql_max_depth` /
/// `graphql_max_complexity` (`--graphql-max-depth` / `MINA_GRAPHQL_MAX_DEPTH`, etc.),
/// defaulting to [`DEFAULT_GRAPHQL_MAX_DEPTH`] / [`DEFAULT_GRAPHQL_MAX_COMPLEXITY`].
///
/// NOTE: complexity here is *structural* (counts each field once) — it does not yet
/// multiply by list `limit:` sizes (that needs per-field complexity annotations, a
/// follow-up); list sizes are bounded today by the per-endpoint pagination caps.
pub fn build_schema(
    store: Arc<IndexerStore>,
    max_depth: usize,
    max_complexity: usize,
) -> Schema<Root, EmptyMutation, EmptySubscription> {
    let mut builder = Schema::build(Root::default(), EmptyMutation, EmptySubscription).data(store);
    if max_depth > 0 {
        builder = builder.limit_depth(max_depth);
    }
    if max_complexity > 0 {
        builder = builder.limit_complexity(max_complexity);
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
}
