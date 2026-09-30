use axum::http::StatusCode;
use chrono::{Duration, SubsecRound, Utc};
use serde_json::json;
use uuid::Uuid;

use hw_bot::db;

use crate::harness::TestApp;

#[tokio::test]
async fn course_teacher_creates_an_assignment_as_a_draft() {
    let app = TestApp::start().await;

    let body = app
        .create_assignment(app.course(), app.teacher())
        .await
        .created();

    assert_eq!(body["status"], json!("draft"));
    assert_eq!(body["published_at"], json!(null));
    assert_eq!(body["teaching_course_id"], json!(app.course().to_string()));
    assert_eq!(body["title"], json!("Rust lifetimes"));
}

#[tokio::test]
async fn a_created_assignment_inherits_the_group_of_its_course() {
    let app = TestApp::start().await;

    let body = app
        .create_assignment(app.course(), app.teacher())
        .await
        .created();

    assert_eq!(
        body["student_group_id"],
        json!(app.fixture.group.id().as_uuid().to_string())
    );
}

#[tokio::test]
async fn publishing_opens_the_assignment_for_submissions() {
    let app = TestApp::start().await;
    let assignment = app.drafted_assignment(app.teacher()).await;

    let body = app.publish_assignment(assignment, app.teacher()).await.ok();

    assert_eq!(body["status"], json!("published"));
    assert!(body["published_at"].as_str().is_some());

    app.submit(
        assignment,
        app.student(),
        "https://github.com/course/hw/pull/1",
    )
    .await
    .created();
}

#[tokio::test]
async fn an_assignment_cannot_be_published_twice() {
    let app = TestApp::start().await;
    let assignment = app.drafted_assignment(app.teacher()).await;
    app.publish_assignment(assignment, app.teacher()).await.ok();

    app.publish_assignment(assignment, app.teacher())
        .await
        .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn an_archived_assignment_can_be_published_again() {
    let app = TestApp::start().await;
    let assignment = app.drafted_assignment(app.teacher()).await;
    app.archive_assignment(assignment, app.teacher()).await.ok();

    let body = app.publish_assignment(assignment, app.teacher()).await.ok();

    assert_eq!(body["status"], json!("published"));
}

#[tokio::test]
async fn a_reopened_assignment_accepts_submissions_again() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    app.publish_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    app.change_deadline(
        app.assignment(),
        app.teacher(),
        Utc::now() + Duration::days(30),
    )
    .await
    .ok();

    app.submit(
        app.assignment(),
        app.student(),
        "https://github.com/course/hw/pull/1",
    )
    .await
    .created();
}

#[tokio::test]
async fn archiving_closes_the_assignment_for_submissions() {
    let app = TestApp::start().await;

    let body = app
        .archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    assert_eq!(body["status"], json!("archived"));

    app.submit(
        app.assignment(),
        app.student(),
        "https://github.com/course/hw/pull/1",
    )
    .await
    .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn a_moved_deadline_applies_to_the_next_submission() {
    let app = TestApp::start().await;
    let past = Utc::now() - Duration::minutes(1);

    let body = app
        .change_deadline(app.assignment(), app.teacher(), past)
        .await
        .ok();

    assert_eq!(body["deadline_at"], json!(past));

    app.submit(
        app.assignment(),
        app.student(),
        "https://github.com/course/hw/pull/1",
    )
    .await
    .error(StatusCode::CONFLICT, "deadline_exceeded");
}

#[tokio::test]
async fn an_archived_assignment_keeps_its_deadline() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    app.change_deadline(
        app.assignment(),
        app.teacher(),
        Utc::now() + Duration::days(1),
    )
    .await
    .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_create_an_assignment() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    app.create_assignment(app.course(), outsider.telegram_user_id().as_i64())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_student_cannot_create_an_assignment() {
    let app = TestApp::start().await;

    app.create_assignment(app.course(), app.student())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_publish_an_assignment() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;
    let assignment = app.drafted_assignment(app.teacher()).await;

    app.publish_assignment(assignment, outsider.telegram_user_id().as_i64())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn an_admin_manages_assignments_of_any_course() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;
    let assignment = app.drafted_assignment(app.teacher()).await;

    let body = app
        .publish_assignment(assignment, admin.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(body["status"], json!("published"));
}

#[tokio::test]
async fn a_second_course_teacher_manages_the_same_assignments() {
    let app = TestApp::start().await;
    let reviewer = app.add_course_teacher("Second teacher").await;

    let body = app
        .create_assignment(app.course(), reviewer.telegram_user_id().as_i64())
        .await
        .created();

    assert_eq!(
        body["created_by"],
        json!(reviewer.id().as_uuid().to_string())
    );
}

#[tokio::test]
async fn an_assignment_cannot_be_created_in_an_unknown_course() {
    let app = TestApp::start().await;

    app.create_assignment(Uuid::now_v7(), app.teacher())
        .await
        .not_found();
}

#[tokio::test]
async fn an_unknown_assignment_cannot_be_published() {
    let app = TestApp::start().await;

    app.publish_assignment(Uuid::now_v7(), app.teacher())
        .await
        .not_found();
}

#[tokio::test]
async fn an_unregistered_actor_cannot_create_an_assignment() {
    let app = TestApp::start().await;

    app.create_assignment(app.course(), 9999).await.not_found();
}

#[tokio::test]
async fn a_published_assignment_reaches_the_review_queue() {
    let app = TestApp::start().await;
    let assignment = app.drafted_assignment(app.teacher()).await;
    app.publish_assignment(assignment, app.teacher()).await.ok();
    let submission = app
        .submit(
            assignment,
            app.student(),
            "https://github.com/course/hw/pull/2",
        )
        .await
        .created();

    let queued = app.take_next(app.teacher()).await.ok();

    assert_eq!(queued["submission"]["id"], submission["id"]);
    assert_eq!(
        queued["assignment"]["id"],
        json!(assignment.to_string()),
        "the queue must carry the assignment the teacher just published"
    );
}

#[tokio::test]
async fn assignment_revision_rejects_a_stale_deadline_update() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let first = db::assignments::find_versioned_by_id(&app.pool, app.fixture.assignment.id())
        .await?
        .expect("assignment");
    let second = db::assignments::find_versioned_by_id(&app.pool, app.fixture.assignment.id())
        .await?
        .expect("assignment");
    let first_revision = first.revision;
    let second_revision = second.revision;
    let now = Utc::now().trunc_subsecs(6);
    let first_deadline = now + Duration::days(10);
    let second_deadline = now + Duration::days(20);
    let mut first_assignment = first.assignment;
    let mut second_assignment = second.assignment;
    first_assignment.change_deadline(&app.fixture.course, &app.fixture.teacher, first_deadline)?;
    second_assignment.change_deadline(
        &app.fixture.course,
        &app.fixture.teacher,
        second_deadline,
    )?;

    db::assignments::update(&app.pool, &first_assignment, first_revision).await?;
    let stale = db::assignments::update(&app.pool, &second_assignment, second_revision).await;

    assert!(matches!(stale, Err(db::DbError::Conflict { .. })));
    let stored = db::assignments::find_by_id(&app.pool, app.fixture.assignment.id())
        .await?
        .expect("assignment");
    assert_eq!(stored.deadline_at(), first_deadline);

    Ok(())
}
