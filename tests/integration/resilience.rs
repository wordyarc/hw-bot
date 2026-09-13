use std::time::Duration;

use axum::http::StatusCode;
use hw_bot::api::REQUEST_BODY_LIMIT;
use sqlx::ConnectOptions;

use crate::harness::TestApp;

#[tokio::test]
async fn malformed_path_uses_the_api_error_envelope() {
    let app = TestApp::start().await;
    app.get_as("/me/assignments/not-a-uuid", app.student())
        .await
        .error(StatusCode::BAD_REQUEST, "invalid_path");
}

#[tokio::test]
async fn unknown_route_uses_the_api_error_envelope() {
    let app = TestApp::start().await;
    app.get("/does-not-exist").await.not_found();
}

#[tokio::test]
async fn unsupported_method_uses_the_api_error_envelope() {
    let app = TestApp::start().await;
    app.post_raw("/health/live", Vec::new())
        .await
        .error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed");
}

#[tokio::test]
async fn http_accepts_a_body_at_the_transport_limit() {
    let app = TestApp::start().await;
    let body = registration_body(&app.group_invite_token().await, REQUEST_BODY_LIMIT);

    let response = app
        .post_raw_as("/students", app.next_telegram_id(), body)
        .await;

    response.created();
}

#[tokio::test]
async fn http_rejects_a_body_above_the_transport_limit() {
    let app = TestApp::start().await;
    let body = registration_body(&app.group_invite_token().await, REQUEST_BODY_LIMIT + 1);

    let response = app
        .post_raw_as("/students", app.next_telegram_id(), body)
        .await;

    response.error(StatusCode::PAYLOAD_TOO_LARGE, "request_body_too_large");
}

#[tokio::test]
async fn http_request_timeout_bounds_waiting_for_the_database() {
    let app = TestApp::start_with_http_timeout(Duration::from_millis(50)).await;
    let mut connections = Vec::new();
    for _ in 0..app.pool.options().get_max_connections() {
        connections.push(app.pool.acquire().await.unwrap());
    }

    let response = app.get("/health").await;

    response.error(StatusCode::REQUEST_TIMEOUT, "request_timeout");
}

#[tokio::test]
async fn an_exhausted_pool_answers_overloaded() {
    let app = TestApp::start_saturated().await;
    let _held = app.pool.acquire().await.unwrap();

    let response = app.get_as("/me", app.student()).await;

    response.error(StatusCode::SERVICE_UNAVAILABLE, "overloaded");
}

#[tokio::test]
async fn runtime_pool_bounds_connection_waiting() {
    let setup_pool = crate::harness::fresh_database().await;
    let database_url = setup_pool.connect_options().to_url_lossy().to_string();
    setup_pool.close().await;
    let pool = hw_bot::db::connect_runtime(
        &database_url,
        1,
        Duration::from_millis(250),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let _connection = pool.acquire().await.unwrap();

    let error = pool.acquire().await.unwrap_err();

    assert!(matches!(error, sqlx::Error::PoolTimedOut));
}

#[tokio::test]
async fn runtime_pool_applies_statement_timeout() {
    let setup_pool = crate::harness::fresh_database().await;
    let database_url = setup_pool.connect_options().to_url_lossy().to_string();
    setup_pool.close().await;
    let pool = hw_bot::db::connect_runtime(
        &database_url,
        1,
        Duration::from_secs(1),
        Duration::from_millis(50),
    )
    .await
    .unwrap();

    let error = sqlx::query("select pg_sleep(1)")
        .execute(&pool)
        .await
        .unwrap_err();

    assert_eq!(
        error.as_database_error().and_then(|error| error.code()),
        Some("57014".into())
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("select 1")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

fn registration_body(invite_token: &str, size: usize) -> Vec<u8> {
    let mut body = serde_json::to_vec(&serde_json::json!({
        "display_name": "Student",
        "invite_token": invite_token,
    }))
    .unwrap();
    assert!(size >= body.len());
    body.resize(size, b' ');
    body
}
