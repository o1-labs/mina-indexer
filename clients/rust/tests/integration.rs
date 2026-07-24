//! Integration tests: the client against a mock HTTP server, so the full
//! reqwest → HTTP → JSON path is exercised without a live indexer.

use mina_indexer_client::{Error, MinaIndexerClient};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    MockServer::start().await
}

fn client(base: &str) -> MinaIndexerClient {
    MinaIndexerClient::new(base)
}

#[tokio::test]
async fn healthz_reflects_status_code() {
    let up = server().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status":"ok"})))
        .mount(&up)
        .await;
    assert!(client(&up.uri()).healthz().await.unwrap());

    let down = server().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&down)
        .await;
    assert!(!client(&down.uri()).healthz().await.unwrap());
}

#[tokio::test]
async fn readyz_ready_and_catching_up() {
    // ready: 200 + ready:true
    let ready = server().await;
    Mock::given(method("GET"))
        .and(path("/readyz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "ready", "ready": true, "tip_height": 538600, "tip_age_seconds": 30, "max_lag_seconds": 600
        })))
        .mount(&ready)
        .await;
    let c = client(&ready.uri());
    assert!(c.is_ready().await.unwrap());
    assert_eq!(c.readyz().await.unwrap().tip_height, Some(538600));

    // catching up: 503 + ready:false — the client still parses the body, not errors
    let behind = server().await;
    Mock::given(method("GET"))
        .and(path("/readyz"))
        .respond_with(ResponseTemplate::new(503).set_body_json(serde_json::json!({
            "status": "catching_up", "ready": false, "tip_height": 528432, "tip_age_seconds": 3_400_000, "max_lag_seconds": 600
        })))
        .mount(&behind)
        .await;
    let c = client(&behind.uri());
    assert!(!c.is_ready().await.unwrap());
    let r = c.readyz().await.unwrap();
    assert_eq!(r.status, "catching_up");
    assert_eq!(r.tip_age_seconds, Some(3_400_000));
}

#[tokio::test]
async fn summary_and_db_version() {
    let s = server().await;
    Mock::given(method("GET"))
        .and(path("/summary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "dbVersion": "0.19.0-493027dc", "blockchainLength": 538600
        })))
        .mount(&s)
        .await;
    let c = client(&s.uri());
    assert_eq!(c.db_version().await.unwrap(), "0.19.0-493027dc");
    assert_eq!(c.summary().await.unwrap()["blockchainLength"], 538600);
}

#[tokio::test]
async fn graphql_data_and_errors() {
    // data -> typed helper
    let ok = server().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "accountsCount": 91691 }
        })))
        .mount(&ok)
        .await;
    assert_eq!(client(&ok.uri()).accounts_count(None).await.unwrap(), 91691);

    // errors -> Error::GraphQl
    let err = server().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "errors": [{ "message": "boom" }]
        })))
        .mount(&err)
        .await;
    let e = client(&err.uri()).tip_height().await.unwrap_err();
    assert!(matches!(e, Error::GraphQl(_)), "expected GraphQl error, got {e:?}");
}

#[tokio::test]
async fn tip_height_from_graphql() {
    let s = server().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "blocks": [ { "blockHeight": 538601 } ] }
        })))
        .mount(&s)
        .await;
    assert_eq!(client(&s.uri()).tip_height().await.unwrap(), 538601);
}
