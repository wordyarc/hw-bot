use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use serde_json::json;

use crate::harness::TestApp;

#[tokio::test]
async fn health_endpoint_checks_the_database() {
    let app = TestApp::start().await;

    app.get("/health").await.expect(StatusCode::OK);
}

#[tokio::test]
async fn liveness_does_not_depend_on_the_database() {
    let app = TestApp::start().await;
    app.pool.close().await;

    app.get("/health/live").await.expect(StatusCode::OK);
    app.get("/health/ready")
        .await
        .expect(StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn echo_returns_the_app_name() {
    let app = TestApp::start().await;

    let response = app.post("/echo", json!({ "text": "ping" })).await;
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    let body = response.ok();

    assert_eq!(body["app"], json!("hw-bot-test"));
    assert_eq!(body["text"], json!("ping"));
}

#[tokio::test]
async fn echo_rejects_a_malformed_body() {
    let app = TestApp::start().await;

    app.post("/echo", json!({ "wrong": "field" }))
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request_body");
}
