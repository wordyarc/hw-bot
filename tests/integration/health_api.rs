use axum::http::StatusCode;

use crate::harness::TestApp;

#[tokio::test]
async fn health_endpoint_checks_the_database() {
    let app = TestApp::start().await;

    app.get("/health").await.expect(StatusCode::OK);
}
