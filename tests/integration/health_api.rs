use axum::http::StatusCode;
use serde_json::json;

use crate::harness::TestApp;

#[tokio::test]
async fn health_endpoint_checks_the_database() {
    let app = TestApp::start().await;

    let body = app.get("/health").await.expect(StatusCode::OK);

    assert_eq!(body, json!(null), "health has no body to return");
}

#[tokio::test]
async fn echo_returns_the_app_name() {
    let app = TestApp::start().await;

    let body = app.post("/echo", json!({ "text": "ping" })).await.ok();

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
