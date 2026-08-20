use axum::http::StatusCode;
use serde_json::{Value, json};

use hw_bot::domain::{InviteGeneration, StudentGroupId};

use crate::harness::TestApp;

#[tokio::test]
async fn the_group_directory_is_no_longer_exposed() {
    let app = TestApp::start().await;

    let response = app.get("/student-groups").await;

    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn student_registers_themselves_with_a_group_invite() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    let token = app.group_invite_token().await;

    let body = app.register_student(2001, "Ivan", &token).await.created();

    assert_eq!(body["role"], json!("student"));
    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["telegram_user_id"], json!(2001));
    assert_eq!(body["student_group"]["code"], json!(code));
    assert!(body["id"].as_str().is_some());
}

#[tokio::test]
async fn repeated_registration_is_a_conflict() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    app.register_student(2001, "Ivan", &token).await.created();

    app.register_student(2001, "Ivan again", &token)
        .await
        .error(StatusCode::CONFLICT, "already_registered");
}

#[tokio::test]
async fn one_invite_registers_several_students() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;

    app.register_student(2101, "Ivan", &token).await.created();
    app.register_student(2102, "Petr", &token).await.created();
}

#[tokio::test]
async fn registration_without_an_invite_is_rejected() {
    let app = TestApp::start().await;

    app.register_student(2002, "Petr", "")
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn a_tampered_invite_is_rejected() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    let tampered = format!("{}A", &token[..token.len() - 1]);

    app.register_student(2002, "Petr", &tampered)
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn a_signed_invite_for_an_unknown_group_is_rejected() {
    let app = TestApp::start().await;
    let token = app
        .invites
        .encode(StudentGroupId::new(), InviteGeneration::FIRST);

    app.register_student(2002, "Petr", &token)
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn a_registered_caller_reads_their_own_profile() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    let token = app.group_invite_token().await;
    app.register_student(2001, "Ivan", &token).await.created();

    let body = app.get_as("/me", 2001).await.ok();

    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["student_group"]["code"], json!(code));
}

/// 404 значит «предложи регистрацию».
#[tokio::test]
async fn an_unregistered_caller_is_not_found() {
    let app = TestApp::start().await;

    app.get_as("/me", 9999).await.not_found();
}

#[tokio::test]
async fn admin_registers_staff() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    let body = app
        .register_staff(
            admin.telegram_user_id().as_i64(),
            2003,
            "New teacher",
            "teacher",
        )
        .await
        .created();

    assert_eq!(body["role"], json!("teacher"));
    assert_eq!(
        body["student_group"],
        Value::Null,
        "staff is never bound to a student group"
    );
}

#[tokio::test]
async fn an_ordinary_teacher_cannot_promote_anyone() {
    let app = TestApp::start().await;

    app.register_staff(app.teacher(), 2004, "Sneaky", "admin")
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_student_cannot_make_themselves_staff() {
    let app = TestApp::start().await;

    app.register_staff(app.student(), 2005, "Sneaky", "teacher")
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn student_is_not_a_staff_role() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    app.register_staff(
        admin.telegram_user_id().as_i64(),
        2006,
        "Not staff",
        "student",
    )
    .await
    .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request_body");
}

#[tokio::test]
async fn a_student_registered_over_http_can_submit_right_away() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    app.register_student(2001, "Ivan", &token).await.created();

    let body = app
        .submit(
            app.assignment(),
            2001,
            "https://github.com/course/hw/pull/1",
        )
        .await
        .created();

    assert_eq!(body["attempt_number"], json!(1));
}
