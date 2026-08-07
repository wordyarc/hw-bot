use axum::http::StatusCode;
use serde_json::json;

use crate::harness::TestApp;

#[tokio::test]
async fn a_command_without_the_actor_header_is_unauthenticated() {
    let app = TestApp::start().await;

    app.post(
        &format!("/assignments/{}/submissions", app.assignment()),
        json!({ "content": "pull/1" }),
    )
    .await
    .error(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn a_read_without_the_actor_header_is_unauthenticated() {
    let app = TestApp::start().await;

    app.get("/me")
        .await
        .error(StatusCode::UNAUTHORIZED, "unauthenticated");
}

#[tokio::test]
async fn a_non_numeric_actor_header_is_rejected() {
    let app = TestApp::start().await;

    app.post_with_actor_header(
        &format!("/assignments/{}/submissions", app.assignment()),
        "ivan",
        json!({ "content": "pull/1" }),
    )
    .await
    .error(StatusCode::BAD_REQUEST, "invalid_actor");
}

#[tokio::test]
async fn a_non_positive_actor_header_is_rejected() {
    let app = TestApp::start().await;

    app.submit(app.assignment(), 0, "pull/1")
        .await
        .error(StatusCode::BAD_REQUEST, "invalid_actor");
}

/// подсунуть чужого актора телом больше нельзя - `deny_unknown_fields`
/// ловит поле раньше
#[tokio::test]
async fn the_actor_is_no_longer_accepted_in_the_body() {
    let app = TestApp::start().await;

    app.submit_raw(
        app.assignment(),
        app.student(),
        json!({ "telegram_user_id": app.teacher(), "content": "pull/1" }),
    )
    .await
    .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request_body");
}

#[tokio::test]
async fn reference_data_is_readable_without_an_actor() {
    let app = TestApp::start().await;

    app.get("/student-groups").await.ok();
}

#[tokio::test]
async fn registration_binds_the_account_to_the_calling_telegram_id() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();

    let body = app.register_student(2100, "Ivan", &code).await.created();

    assert_eq!(body["telegram_user_id"], json!(2100));
}
