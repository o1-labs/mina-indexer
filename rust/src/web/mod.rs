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
use actix_governor::{Governor, GovernorConfigBuilder};
use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    error::ErrorPayloadTooLarge,
    guard, http,
    middleware::{self, from_fn, Condition, Next},
    web,
    web::Data,
    App, Error, HttpServer,
};
use async_graphql_actix_web::GraphQL;
use log::{info, warn};
use std::{net, sync::Arc, time::Duration};
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

/// Max accepted request body size, injected as app data so the generic
/// [`enforce_body_limit`] middleware can read it. `usize::MAX` = no cap.
#[derive(Clone, Copy)]
struct BodyLimit(usize);

/// Reject oversized requests with HTTP 413 based on the `Content-Length` header,
/// before the handler reads the body. This covers the GraphQL POST endpoint,
/// which actix's `PayloadConfig` does *not* bound (async-graphql reads the body
/// itself). Bodies sent without a `Content-Length` (chunked) bypass this — the
/// reverse proxy is the backstop for those (see ops/reverse-proxy/).
async fn enforce_body_limit<B: MessageBody>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<B>, Error> {
    if let Some(&BodyLimit(max)) = req.app_data::<BodyLimit>() {
        let len = req
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok());
        if let Some(len) = len {
            if len > max {
                return Err(ErrorPayloadTooLarge(
                    "request body exceeds the configured maximum (--web-max-body-bytes)",
                ));
            }
        }
    }
    next.call(req).await
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

/// Web-server tunables (GraphQL DoS guards + CORS). Bundled so `start_web_server`
/// stays under the argument-count lint as more knobs are added.
pub struct WebServerConfig {
    pub graphql_max_depth: usize,
    pub graphql_max_complexity: usize,
    pub graphql_timeout_secs: u64,
    pub graphql_disable_introspection: bool,
    pub cors_allowed_origins: Vec<String>,
    /// Seconds to wait for client request headers before dropping the
    /// connection (`0` disables).
    pub request_timeout_secs: u64,
    /// Max accepted request body size in bytes (`0` disables the cap).
    pub max_body_bytes: usize,
    /// Sustained per-IP request rate (requests/second). Rate limiting is off
    /// unless this and `rate_limit_burst` are both > 0.
    pub rate_limit_per_second: u64,
    /// Per-IP burst allowance (requests) for the rate limiter.
    pub rate_limit_burst: u32,
}

pub async fn start_web_server<A: net::ToSocketAddrs>(
    subsys: SubsystemHandle,
    state: Arc<IndexerStore>,
    addrs: A,
    config: WebServerConfig,
) -> anyhow::Result<()> {
    let WebServerConfig {
        graphql_max_depth,
        graphql_max_complexity,
        graphql_timeout_secs,
        graphql_disable_introspection,
        cors_allowed_origins,
        request_timeout_secs,
        max_body_bytes,
        rate_limit_per_second,
        rate_limit_burst,
    } = config;

    let locked = Arc::new(load_locked_balances());
    crate::metrics::init();

    // Per-IP rate limiting (off unless both knobs are > 0). Built regardless so
    // the middleware type is fixed; `Condition` skips applying it when disabled.
    let rate_limit_enabled = rate_limit_per_second > 0 && rate_limit_burst > 0;
    if rate_limit_enabled {
        info!(
            "Rate limiting: {rate_limit_per_second} req/s per IP, burst {rate_limit_burst}"
        );
    }
    let governor_conf = GovernorConfigBuilder::default()
        .requests_per_second(if rate_limit_enabled {
            rate_limit_per_second
        } else {
            1
        })
        .burst_size(if rate_limit_enabled { rate_limit_burst } else { 1 })
        .finish()
        .expect("valid governor config (non-zero rate + burst)");

    if cors_allowed_origins.is_empty() {
        warn!(
            "CORS is wildcard-open (Access-Control-Allow-Origin: *). \
             Set --web-cors-allowed-origins (or MINA_WEB_CORS_ALLOWED_ORIGINS) \
             to restrict cross-origin access on public deployments."
        );
    }
    let cors_allowed_origins = Arc::new(cors_allowed_origins);

    // `0` disables the cap; actix's PayloadConfig needs a concrete ceiling.
    let body_limit = if max_body_bytes == 0 {
        usize::MAX
    } else {
        max_body_bytes
    };

    let _ = HttpServer::new(move || {
        App::new()
            .app_data(Data::new(state.clone()))
            .app_data(Data::new(locked.clone()))
            .app_data(BodyLimit(body_limit))
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
            // Reject oversized bodies (HTTP 413) via Content-Length.
            .wrap(from_fn(enforce_body_limit))
            // Registered last ⇒ runs first: rate-limited requests are rejected
            // (HTTP 429) before any other processing. No-op when disabled.
            .wrap(Condition::new(
                rate_limit_enabled,
                Governor::new(&governor_conf),
            ))
    })
    // Drop connections that stall before sending their request headers. A zero
    // duration disables the timeout (our `0` = disabled).
    .client_request_timeout(Duration::from_secs(request_timeout_secs))
    .bind(addrs)
    .unwrap()
    .run()
    .cancel_on_shutdown(&subsys)
    .await;

    Ok(())
}
