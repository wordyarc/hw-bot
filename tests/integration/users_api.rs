use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::harness::TestApp;

#[tokio::test]
async fn student_registers_themselves_with_a_course_invite() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    let token = app.course_invite_token().await;

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
    let token = app.course_invite_token().await;
    app.register_student(2001, "Ivan", &token).await.created();

    app.register_student(2001, "Ivan again", &token)
        .await
        .error(StatusCode::CONFLICT, "already_registered");
}

#[tokio::test]
async fn one_invite_registers_several_students() {
    let app = TestApp::start().await;
    let token = app.course_invite_token().await;

    app.register_student(2101, "Ivan", &token).await.created();
    app.register_student(2102, "Petr", &token).await.created();
}

#[tokio::test]
async fn a_tampered_invite_is_rejected() {
    let app = TestApp::start().await;
    let token = app.course_invite_token().await;
    let replacement = if token.ends_with('A') { 'B' } else { 'A' };
    let tampered = format!("{}{replacement}", &token[..token.len() - 1]);
    assert_ne!(tampered, token);

    app.register_student(2002, "Petr", &tampered)
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn a_student_registers_into_the_group_chosen_from_a_course_of_several_groups() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let token = app.course_invite_token().await;

    let body = app
        .register_student_in(2001, "Ivan", &token, second.id())
        .await
        .created();

    assert_eq!(body["student_group"]["code"], json!(second.code().as_str()));
}

#[tokio::test]
async fn a_course_of_several_groups_needs_the_chosen_group() {
    let app = TestApp::start().await;
    app.add_group_to_course(&app.fixture.course).await;
    let token = app.course_invite_token().await;

    app.register_student(2001, "Ivan", &token)
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input");
    app.get_as("/me", 2001).await.not_found();
}

#[tokio::test]
async fn a_registered_caller_reads_their_own_profile() {
    let app = TestApp::start().await;
    let code = app.fixture.group.code().as_str().to_owned();
    let token = app.course_invite_token().await;
    app.register_student(2001, "Ivan", &token).await.created();

    let body = app.get_as("/me", 2001).await.ok();

    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["student_group"]["code"], json!(code));
}

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
async fn a_student_registered_over_http_can_submit_right_away() {
    let app = TestApp::start().await;
    let token = app.course_invite_token().await;
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

fn listed_titles(body: &Value) -> Vec<&str> {
    body.as_array()
        .unwrap()
        .iter()
        .map(|item| item["assignment"]["title"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn a_registration_without_course_ids_enrolls_in_the_course_of_the_invite() {
    let app = TestApp::start().await;
    let second = app.add_subject_course("Android").await;
    app.add_published_assignment(&second, &app.fixture.teacher, "Activity lifecycle")
        .await;
    let token = app.course_invite_token().await;

    app.register_student(2001, "Ivan", &token).await.created();

    let body = app.student_assignments(2001).await.ok();
    assert_eq!(
        listed_titles(&body),
        [app.fixture.assignment.title().as_str()]
    );
}

#[tokio::test]
async fn a_registration_with_course_ids_enrolls_in_exactly_those_courses() {
    let app = TestApp::start().await;
    let second = app.add_subject_course("Android").await;
    app.add_published_assignment(&second, &app.fixture.teacher, "Activity lifecycle")
        .await;
    let token = app.course_invite_token().await;

    app.post_as(
        "/students",
        2001,
        json!({
            "display_name": "Ivan",
            "invite_token": token,
            "teaching_course_ids": [second.id().as_uuid()],
        }),
    )
    .await
    .created();

    let body = app.student_assignments(2001).await.ok();
    assert_eq!(listed_titles(&body), ["Activity lifecycle"]);
}

#[tokio::test]
async fn a_course_outside_the_group_among_the_course_ids_is_invalid_input() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await.course;
    let token = app.course_invite_token().await;

    app.post_as(
        "/students",
        2001,
        json!({
            "display_name": "Ivan",
            "invite_token": token,
            "teaching_course_ids": [foreign.id().as_uuid()],
        }),
    )
    .await
    .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input");
    app.get_as("/me", 2001).await.not_found();
}
