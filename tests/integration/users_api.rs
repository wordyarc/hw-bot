use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::harness::TestApp;

#[tokio::test]
async fn student_groups_are_listed_for_the_registration_picker() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str();

    let body = app.get("/student-groups").await.ok();

    let group = body
        .as_array()
        .expect("a json array")
        .iter()
        .find(|group| group["code"] == json!(code))
        .unwrap_or_else(|| panic!("group {code} is missing in {body:#}"));

    assert_eq!(group["education_level"], json!("bachelor"));
    assert_eq!(group["study_year"], json!(1));
}

#[tokio::test]
async fn student_registers_themselves() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();

    let body = app.register_student(2001, "Ivan", &code).await.created();

    assert_eq!(body["role"], json!("student"));
    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["telegram_user_id"], json!(2001));
    assert_eq!(body["student_group"]["code"], json!(code));
    assert!(body["id"].as_str().is_some());
}

#[tokio::test]
async fn repeated_registration_is_a_conflict() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    app.register_student(2001, "Ivan", &code).await.created();

    app.register_student(2001, "Ivan again", &code)
        .await
        .error(StatusCode::CONFLICT, "user_already_registered");
}

#[tokio::test]
async fn registration_into_unknown_group_is_not_found() {
    let app = TestApp::start().await;

    app.register_student(2002, "Petr", "NO-SUCH-GROUP")
        .await
        .not_found();
}

#[tokio::test]
async fn empty_display_name_is_rejected() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();

    app.register_student(2002, "   ", &code)
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "empty_field");
}

#[tokio::test]
async fn a_registered_caller_reads_their_own_profile() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    app.register_student(2001, "Ivan", &code).await.created();

    let body = app.get_as("/me", 2001).await.ok();

    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["student_group"]["code"], json!(code));
}

/// Для бота это не ошибка, а развилка: 404 значит «предложи регистрацию».
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
    let code = app.fixture.group.code().as_str().to_owned();
    app.register_student(2001, "Ivan", &code).await.created();

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
