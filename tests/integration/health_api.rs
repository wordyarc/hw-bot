use axum::http::StatusCode;

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
