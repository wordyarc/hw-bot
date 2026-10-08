use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::harness::TestApp;

#[tokio::test]
async fn student_submits_published_assignment() {
    let app = TestApp::start().await;

    let body = app
        .submit(
            app.assignment(),
            app.student(),
            "  https://github.com/course/hw/pull/1  ",
        )
        .await
        .created();

    assert_eq!(body["assignment_id"], json!(app.assignment()));
    assert_eq!(
        body["student_id"],
        json!(app.fixture.student.id().as_uuid())
    );
    assert_eq!(body["attempt_number"], json!(1));
    assert_eq!(body["status"], json!("pending_review"));
    assert_eq!(
        body["content"],
        json!("https://github.com/course/hw/pull/1"),
        "content is trimmed by the domain before it is stored"
    );
    assert!(body["id"].as_str().is_some());
    assert!(body["submitted_at"].as_str().is_some());
}

#[tokio::test]
async fn pending_submission_blocks_the_next_attempt() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;

    app.submit(app.assignment(), app.student(), "pull/2")
        .await
        .error(StatusCode::CONFLICT, "submission_pending_review");
}

#[tokio::test]
async fn submitting_to_unknown_assignment_is_not_found() {
    let app = TestApp::start().await;

    app.submit(Uuid::now_v7(), app.student(), "pull/1")
        .await
        .not_found();
}

#[tokio::test]
async fn submitting_as_unknown_user_is_not_found() {
    let app = TestApp::start().await;

    app.submit(app.assignment(), 9999, "pull/1")
        .await
        .not_found();
}

#[tokio::test]
async fn student_from_another_group_cannot_submit() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_student().await;

    app.submit(
        app.assignment(),
        outsider.telegram_user_id().as_i64(),
        "pull/1",
    )
    .await
    .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn teacher_cannot_submit_homework() {
    let app = TestApp::start().await;

    app.submit(app.assignment(), app.teacher(), "pull/1")
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn draft_assignment_cannot_be_submitted() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;

    app.submit(draft.id().as_uuid(), app.student(), "pull/1")
        .await
        .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn malformed_body_uses_the_same_error_envelope_as_handlers() {
    let app = TestApp::start().await;

    app.submit_raw(app.assignment(), app.student(), json!({}))
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request_body");
}

#[tokio::test]
async fn an_empty_makeup_pool_is_a_conflict_with_its_own_code() {
    let app = TestApp::start().await;
    let first = app.add_closed_assignment("First", 10, 6).await;
    let second = app.add_closed_assignment("Second", 10, 6).await;
    app.enable_makeup(1).await;

    let spent = app
        .submit(first.id().as_uuid(), app.student(), "pull/makeup")
        .await
        .created();
    assert_eq!(spent["kind"], json!("makeup"));

    app.submit(second.id().as_uuid(), app.student(), "pull/makeup")
        .await
        .error(StatusCode::CONFLICT, "makeup_attempts_exhausted");
}
