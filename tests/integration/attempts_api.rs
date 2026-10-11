use axum::http::StatusCode;
use hw_bot::app::{Actor, ErrorCode, GrantExtraAttempts};
use hw_bot::db;
use hw_bot::domain::UserId;
use serde_json::json;

use crate::harness::{TestApp, submission_id};

#[tokio::test]
async fn course_teacher_grants_extra_attempts() {
    let app = TestApp::start().await;

    let body = app
        .grant_extra_attempts(
            app.assignment(),
            app.fixture.student.id().as_uuid(),
            app.teacher(),
            1,
        )
        .await
        .ok();

    assert_eq!(body["assignment_id"], json!(app.assignment()));
    assert_eq!(
        body["student_id"],
        json!(app.fixture.student.id().as_uuid())
    );
    assert_eq!(body["extra_attempts"], json!(1));
}

#[tokio::test]
async fn extra_attempt_grants_are_additive() {
    let app = TestApp::start().await;

    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        app.teacher(),
        2,
    )
    .await
    .ok();

    let body = app
        .grant_extra_attempts(
            app.assignment(),
            app.fixture.student.id().as_uuid(),
            app.teacher(),
            3,
        )
        .await
        .ok();

    assert_eq!(body["extra_attempts"], json!(5));
}

#[tokio::test]
async fn admin_grants_attempts_for_any_course() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        admin.telegram_user_id().as_i64(),
        1,
    )
    .await
    .ok();
}

#[tokio::test]
async fn outsider_teacher_cannot_grant_attempts() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        outsider.telegram_user_id().as_i64(),
        1,
    )
    .await
    .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn an_outsider_is_refused_before_the_student_is_looked_up() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    let error = app
        .application
        .assignments
        .grant_extra_attempts(GrantExtraAttempts {
            assignment_id: app.fixture.assignment.id(),
            student_id: UserId::new(),
            actor: Actor::new(outsider.id()),
            amount: 1,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn student_cannot_grant_attempts() {
    let app = TestApp::start().await;

    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        app.student(),
        1,
    )
    .await
    .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn student_from_another_group_cannot_receive_attempts() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_student().await;

    app.grant_extra_attempts(app.assignment(), outsider.id().as_uuid(), app.teacher(), 1)
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn extra_attempt_unlocks_the_next_submission() {
    let app = TestApp::start().await;

    for attempt in 1..=2 {
        let body = app
            .submit(app.assignment(), app.student(), &format!("pull/{attempt}"))
            .await
            .created();
        app.review(submission_id(&body), app.teacher(), "reject", "Try again.")
            .await
            .ok();
    }

    app.submit(app.assignment(), app.student(), "pull/3")
        .await
        .error(StatusCode::CONFLICT, "attempts_exhausted");

    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        app.teacher(),
        1,
    )
    .await
    .ok();

    let body = app
        .submit(app.assignment(), app.student(), "pull/3")
        .await
        .created();
    assert_eq!(body["attempt_number"], json!(3));
}

#[tokio::test]
async fn grant_is_visible_in_student_assignment_progress() {
    let app = TestApp::start().await;
    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        app.teacher(),
        1,
    )
    .await
    .ok();

    let body = app.student_assignments(app.student()).await.ok();

    assert_eq!(body[0]["progress"]["allowed_attempts"], json!(3));
    assert_eq!(body[0]["progress"]["remaining_attempts"], json!(3));
}

#[tokio::test]
async fn concurrent_grants_are_not_lost() {
    let app = TestApp::start().await;
    let student_id = app.fixture.student.id();

    let (first, second) = tokio::join!(
        app.grant_extra_attempts(app.assignment(), student_id.as_uuid(), app.teacher(), 1),
        app.grant_extra_attempts(app.assignment(), student_id.as_uuid(), app.teacher(), 1),
    );

    first.ok();
    second.ok();

    let history = db::submissions::load_histories(
        &app.pool,
        &[app.fixture.assignment.id().as_uuid()],
        student_id,
    )
    .await
    .expect("load submission history")
    .remove(&app.fixture.assignment.id())
    .expect("requested assignment history");
    assert_eq!(history.extra_attempts(), 2);
}
