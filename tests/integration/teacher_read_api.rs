use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::harness::TestApp;

#[track_caller]
fn ids(body: &Value) -> Vec<String> {
    body.as_array()
        .expect("a json array")
        .iter()
        .map(|item| item["id"].as_str().expect("an id").to_owned())
        .collect()
}

#[tokio::test]
async fn a_teacher_sees_their_course_with_subject_and_group() {
    let app = TestApp::start().await;

    let body = app.get_as("/me/courses", app.teacher()).await.ok();

    let items = body.as_array().expect("a json array");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], json!(app.course()));
    assert_eq!(items[0]["subject"]["name"], json!("Backend development"));
    assert_eq!(
        items[0]["student_group"]["code"],
        json!(app.fixture.group.code().as_str())
    );
    assert_eq!(
        items[0]["student_group"]["education_level"],
        json!("bachelor")
    );
}

#[tokio::test]
async fn a_teacher_does_not_see_courses_they_do_not_teach() {
    let app = TestApp::start().await;
    let (foreign_teacher, _, _) = app.add_foreign_course_assignment().await;

    let body = app
        .get_as("/me/courses", foreign_teacher.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(
        ids(&body).len(),
        1,
        "only their own course, not the fixture one"
    );
    assert_ne!(ids(&body)[0], app.course().to_string());
}

#[tokio::test]
async fn an_admin_sees_every_course() {
    let app = TestApp::start().await;
    app.add_foreign_course_assignment().await;
    let admin = app.add_admin().await;

    let body = app
        .get_as("/me/courses", admin.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(ids(&body).len(), 2);
}

#[tokio::test]
async fn a_second_course_teacher_sees_the_shared_course() {
    let app = TestApp::start().await;
    let reviewer = app.add_course_teacher("Second teacher").await;

    let body = app
        .get_as("/me/courses", reviewer.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(ids(&body), vec![app.course().to_string()]);
    assert_eq!(body[0]["teacher_count"], json!(2));
}

#[tokio::test]
async fn a_student_has_no_courses_to_teach() {
    let app = TestApp::start().await;

    app.get_as("/me/courses", app.student())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_teacher_sees_drafts_and_archived_assignments_of_their_course() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;

    let body = app
        .get_as(
            &format!("/teaching-courses/{}/assignments", app.course()),
            app.teacher(),
        )
        .await
        .ok();

    let listed = ids(&body);
    assert!(listed.contains(&draft.id().as_uuid().to_string()));
    assert!(listed.contains(&app.assignment().to_string()));
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_list_its_assignments() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    app.get_as(
        &format!("/teaching-courses/{}/assignments", app.course()),
        outsider.telegram_user_id().as_i64(),
    )
    .await
    .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn assignments_of_an_unknown_course_are_not_found() {
    let app = TestApp::start().await;

    app.get_as(
        &format!("/teaching-courses/{}/assignments", Uuid::now_v7()),
        app.teacher(),
    )
    .await
    .not_found("teaching course not found");
}

#[tokio::test]
async fn an_empty_queue_reports_zero() {
    let app = TestApp::start().await;

    let body = app.get_as("/me/review-queue", app.teacher()).await.ok();

    assert_eq!(body["pending"], json!(0));
}

#[tokio::test]
async fn the_queue_size_counts_only_pending_submissions() {
    let app = TestApp::start().await;
    let second_student = app.add_student("Second student").await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    app.queued_submission(second_student.telegram_user_id().as_i64(), "pull/2")
        .await;
    app.review(first, app.teacher(), "accept", "95/100")
        .await
        .ok();

    let body = app.get_as("/me/review-queue", app.teacher()).await.ok();

    assert_eq!(body["pending"], json!(1));
}

#[tokio::test]
async fn the_queue_size_does_not_count_other_courses() {
    let app = TestApp::start().await;
    let (_, foreign_student, foreign_assignment) = app.add_foreign_course_assignment().await;
    app.submit(
        foreign_assignment.id().as_uuid(),
        foreign_student.telegram_user_id().as_i64(),
        "pull/1",
    )
    .await
    .created();

    let body = app.get_as("/me/review-queue", app.teacher()).await.ok();

    assert_eq!(body["pending"], json!(0));
}

#[tokio::test]
async fn a_student_has_no_review_queue() {
    let app = TestApp::start().await;

    app.get_as("/me/review-queue", app.student())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}
