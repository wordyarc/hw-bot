use std::time::Duration;

use axum::http::StatusCode;
use hw_bot::db::DbError;
use sqlx::{ConnectOptions, PgPool};

use crate::harness::TestApp;

#[tokio::test]
async fn authenticated_routes_use_api_path_and_pagination_rejections() {
    let app = TestApp::start().await;
    for (path, code) in [
        ("/me/assignments/not-a-uuid", "invalid_path"),
        ("/me/assignments?page=-1", "invalid_query"),
    ] {
        app.get_as(path, app.student())
            .await
            .error(StatusCode::BAD_REQUEST, code);
    }
}

#[tokio::test]
async fn http_request_timeout_bounds_waiting_for_the_database() {
    let app = TestApp::start_with_http_timeout(Duration::from_millis(50)).await;
    let mut connections = Vec::new();
    for _ in 0..app.pool.options().get_max_connections() {
        connections.push(app.pool.acquire().await.unwrap());
    }

    let response = app.get("/health/ready").await;

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
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let _connection = pool.acquire().await.unwrap();

    let error = pool.acquire().await.unwrap_err();

    assert!(matches!(error, sqlx::Error::PoolTimedOut));
}

async fn statement_timeout_pool() -> PgPool {
    let setup_pool = crate::harness::fresh_database().await;
    let database_url = setup_pool.connect_options().to_url_lossy().to_string();
    setup_pool.close().await;

    hw_bot::db::connect_runtime(
        &database_url,
        1,
        Duration::from_secs(1),
        Duration::from_millis(50),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn runtime_pool_applies_statement_timeout() {
    let pool = statement_timeout_pool().await;

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

#[tokio::test]
async fn a_statement_timeout_counts_as_database_overload() {
    let pool = statement_timeout_pool().await;

    let error = sqlx::query("select pg_sleep(1)")
        .execute(&pool)
        .await
        .unwrap_err();

    assert!(matches!(DbError::from(error), DbError::Overloaded));
}
