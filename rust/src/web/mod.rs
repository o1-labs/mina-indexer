mod common;
pub mod graphql;
pub mod rest;

pub const ENDPOINT_GRAPHQL: &str = "/graphql";

use self::{
    graphql::{build_schema, indexer_graphiql},
    rest::{accounts, blockchain, blocks, locked_balances::LockedBalances, metrics},
};
use crate::store::IndexerStore;
use actix_cors::Cors;
use actix_web::{guard, http, middleware, web, web::Data, App, HttpServer};
use async_graphql_actix_web::GraphQL;
use log::warn;
use std::{net, sync::Arc};
use tokio_graceful_shutdown::{FutureExt, SubsystemHandle};

/// Build the CORS middleware. With an explicit allow-list, only those origins
/// may make cross-origin (browser) requests; with an empty list the server is
/// wildcard-open (`Cors::permissive`) for backward compatibility — a warning is
/// logged once at startup so this isn't silently the case in production.
fn build_cors(allowed_origins: &[String]) -> Cors {
    if allowed_origins.is_empty() {
        return Cors::permissive();
    }
    let mut cors = Cors::default()
        .allowed_methods([http::Method::GET, http::Method::POST])
        .allow_any_header()
        .max_age(3600);
    for origin in allowed_origins {
        cors = cors.allowed_origin(origin);
    }
    cors
}

fn load_locked_balances() -> LockedBalances {
    match LockedBalances::new() {
        Ok(locked_balances) => locked_balances,
        Err(e) => {
            warn!("locked supply csv ingestion failed. {}", e);
            LockedBalances::default()
        }
    }
}

pub async fn start_web_server<A: net::ToSocketAddrs>(
    subsys: SubsystemHandle,
    state: Arc<IndexerStore>,
    addrs: A,
    graphql_max_depth: usize,
    graphql_max_complexity: usize,
    graphql_timeout_secs: u64,
    graphql_disable_introspection: bool,
    cors_allowed_origins: Vec<String>,
) -> anyhow::Result<()> {
    let locked = Arc::new(load_locked_balances());
    crate::metrics::init();

    if cors_allowed_origins.is_empty() {
        warn!(
            "CORS is wildcard-open (Access-Control-Allow-Origin: *). \
             Set --web-cors-allowed-origins (or MINA_WEB_CORS_ALLOWED_ORIGINS) \
             to restrict cross-origin access on public deployments."
        );
    }
    let cors_allowed_origins = Arc::new(cors_allowed_origins);

    let _ = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(state.clone()))
            .app_data(Data::new(locked.clone()))
            .service(blocks::get_blocks)
            .service(blocks::get_block_by_state_hash)
            .service(accounts::get_account)
            .service(blockchain::get_blockchain_summary)
            .service(blockchain::get_health)
            .service(metrics::get_metrics)
            .service(
                web::resource(ENDPOINT_GRAPHQL)
                    .guard(guard::Post())
                    .to(GraphQL::new(build_schema(
                        state.clone(),
                        graphql_max_depth,
                        graphql_max_complexity,
                        graphql_timeout_secs,
                        graphql_disable_introspection,
                    ))),
            )
            .service(
                web::resource(ENDPOINT_GRAPHQL)
                    .guard(guard::Get())
                    .to(indexer_graphiql),
            )
            .wrap(build_cors(&cors_allowed_origins))
            .wrap(middleware::Logger::default())
    })
    .bind(addrs)
    .unwrap()
    .run()
    .cancel_on_shutdown(&subsys)
    .await;

    Ok(())
}
