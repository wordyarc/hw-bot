use axum::http::StatusCode;
use hw_bot::app::{Actor, AddAssignmentFile};
use hw_bot::domain::FilePresentation;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::harness::{TestApp, insert_assignment, wall_clock_assignment};

#[track_caller]
fn only(body: &Value) -> &Value {
    let items = body.as_array().expect("a json array");
    assert_eq!(items.len(), 1, "expected exactly one item in {body:#}");

    &items[0]
}

#[tokio::test]
async fn a_student_sees_the_published_assignments_of_their_group() {
    let app = TestApp::start().await;

    let body = app.student_assignments(app.student()).await.ok();

    let item = only(&body);
    assert_eq!(item["assignment"]["id"], json!(app.assignment()));
    assert_eq!(item["assignment"]["title"], json!("Rust ownership"));
}

#[tokio::test]
async fn a_student_of_a_group_the_assignment_is_not_issued_to_never_sees_it() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let course = app.reload_course(&app.fixture.course).await;
    let outsider = app
        .add_student_to_group("Second group", &second)
        .await
        .telegram_user_id()
        .as_i64();
    let current = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("First group only")
        .groups([&app.fixture.group])
        .published()
        .build();
    insert_assignment(&app.pool, &current).await;
    let archived = app
        .add_published_assignment(&app.fixture.course, &app.fixture.teacher, "Old")
        .await;
    app.archive_assignment(archived.id().as_uuid(), app.teacher())
        .await
        .ok();

    let listed = app.student_assignments(outsider).await.ok();
    let archive = app.archived_student_assignments(outsider).await.ok();

    assert_eq!(listed, json!([]));
    assert_eq!(archive, json!([]));
    app.student_assignment(current.id().as_uuid(), outsider)
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn the_student_list_names_the_subject_and_groups_assignments_by_it() {
    let app = TestApp::start().await;
    let algorithms = app.add_subject_course("Algorithms").await;
    app.add_published_assignment(&algorithms, &app.fixture.teacher, "Sorting")
        .await;

    let assignments = app
        .application
        .assignments
        .list_for_student(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the student list is readable")
        .items;

    let listed: Vec<_> = assignments
        .iter()
        .map(|item| {
            (
                item.subject.name().as_str(),
                item.assignment.title().as_str(),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("Algorithms", "Sorting"),
            ("Backend development", "Rust ownership"),
        ]
    );
}

#[tokio::test]
async fn a_draft_assignment_stays_out_of_the_student_list() {
    let app = TestApp::start().await;
    app.add_draft_assignment().await;

    let body = app.student_assignments(app.student()).await.ok();

    assert_eq!(only(&body)["assignment"]["id"], json!(app.assignment()));
}

#[tokio::test]
async fn an_archived_assignment_leaves_the_student_list() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    let body = app.student_assignments(app.student()).await.ok();

    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn a_previously_published_assignment_appears_in_the_student_archive() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    let body = app.archived_student_assignments(app.student()).await.ok();

    let item = only(&body);
    assert_eq!(item["assignment"]["id"], json!(app.assignment()));
    assert_eq!(item["assignment"]["status"], json!("archived"));
    assert!(item["assignment"]["published_at"].is_string());
}

#[tokio::test]
async fn a_never_published_assignment_stays_out_of_the_student_archive() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    app.archive_assignment(draft.id().as_uuid(), app.teacher())
        .await
        .ok();

    let body = app.archived_student_assignments(app.student()).await.ok();

    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn assignments_of_another_group_are_invisible() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_student().await;

    let body = app
        .student_assignments(outsider.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn an_untouched_assignment_reports_a_full_quota() {
    let app = TestApp::start().await;

    let body = app.student_assignments(app.student()).await.ok();

    let progress = &only(&body)["progress"];
    assert_eq!(progress["used_attempts"], json!(0));
    assert_eq!(progress["allowed_attempts"], json!(2));
    assert_eq!(progress["remaining_attempts"], json!(2));
    assert_eq!(progress["has_pending_submission"], json!(false));
    assert_eq!(progress["has_accepted_submission"], json!(false));
}

#[tokio::test]
async fn a_pending_submission_shows_up_in_the_list_without_spending_an_attempt() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;

    let body = app.student_assignments(app.student()).await.ok();

    let progress = &only(&body)["progress"];
    assert_eq!(progress["has_pending_submission"], json!(true));
    assert_eq!(progress["used_attempts"], json!(0));
    assert_eq!(progress["remaining_attempts"], json!(2));
}

#[tokio::test]
async fn a_rejection_spends_an_attempt_in_the_list() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "reject", "Add tests.")
        .await
        .ok();

    let body = app.student_assignments(app.student()).await.ok();

    let progress = &only(&body)["progress"];
    assert_eq!(progress["used_attempts"], json!(1));
    assert_eq!(progress["remaining_attempts"], json!(1));
    assert_eq!(progress["has_pending_submission"], json!(false));
}

#[tokio::test]
async fn an_acceptance_is_visible_in_the_list() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "95/100")
        .await
        .ok();

    let body = app.student_assignments(app.student()).await.ok();

    assert_eq!(
        only(&body)["progress"]["has_accepted_submission"],
        json!(true)
    );
}

#[tokio::test]
async fn the_history_endpoint_carries_attempts_with_reviews_newest_first() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    app.review(first, app.teacher(), "reject", "Add tests.")
        .await
        .ok();
    app.queued_submission(app.student(), "pull/2").await;

    let body = app
        .get_as(
            &format!("/me/assignments/{}/submissions", app.assignment()),
            app.student(),
        )
        .await
        .ok();

    let submissions = body.as_array().expect("a json array");
    assert_eq!(submissions.len(), 2);
    assert_eq!(submissions[1]["attempt_number"], json!(1));
    assert_eq!(submissions[1]["review_comment"], json!("Add tests."));
    assert_eq!(submissions[0]["attempt_number"], json!(2));
    assert_eq!(submissions[0]["status"], json!("pending_review"));
}

#[tokio::test]
async fn the_card_of_an_untouched_assignment_has_no_submissions() {
    let app = TestApp::start().await;

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    assert_eq!(body["latest_submission"], json!(null));
    assert_eq!(body["files"], json!([]));
    assert_eq!(
        body["assignment"]["body"],
        json!("Send a pull request link.")
    );
}

#[tokio::test]
async fn the_card_carries_assignment_file_metadata_without_content() {
    let app = TestApp::start().await;
    let assignment = app.fixture.assignment.id();
    let card = app
        .application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), assignment)
        .await
        .unwrap();
    let added = app
        .application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: assignment,
            actor: app.teacher_actor(),
            expected_revision: card.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: b"read the notes".to_vec(),
        })
        .await
        .unwrap();

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    assert_eq!(body["files"][0]["id"], json!(added.files[0].id().as_uuid()));
    assert_eq!(body["files"][0]["position"], json!(1));
    assert_eq!(body["files"][0]["presentation"], json!("document"));
    assert_eq!(body["files"][0]["filename"], json!("assignment-note.txt"));
    assert_eq!(body["files"][0]["mime_type"], json!("text/plain"));
    assert_eq!(body["files"][0]["size_bytes"], json!(14));
    assert!(body["files"][0].get("content").is_none());
    assert!(body["files"][0].get("sha256").is_none());
}

#[tokio::test]
async fn the_card_survives_archiving() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    assert_eq!(body["assignment"]["status"], json!("archived"));
}

#[tokio::test]
async fn an_archived_card_keeps_submission_history_and_review_comments() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "reject", "Add tests.")
        .await
        .ok();
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    assert_eq!(
        body["latest_submission"]["review_comment"],
        json!("Add tests.")
    );
}

#[tokio::test]
async fn a_draft_card_does_not_exist_for_a_student() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;

    app.student_assignment(draft.id().as_uuid(), app.student())
        .await
        .not_found();
}

#[tokio::test]
async fn a_never_published_archived_card_does_not_exist_for_a_student() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    app.archive_assignment(draft.id().as_uuid(), app.teacher())
        .await
        .ok();

    app.student_assignment(draft.id().as_uuid(), app.student())
        .await
        .not_found();
}

#[tokio::test]
async fn a_card_of_another_group_is_forbidden() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_student().await;

    app.student_assignment(app.assignment(), outsider.telegram_user_id().as_i64())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn an_unknown_card_is_not_found() {
    let app = TestApp::start().await;

    app.student_assignment(Uuid::now_v7(), app.student())
        .await
        .not_found();
}

#[tokio::test]
async fn a_teacher_has_no_student_assignments() {
    let app = TestApp::start().await;

    app.student_assignments(app.teacher())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_makeup_leaves_the_regular_attempts_of_the_progress_alone() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    let rejected = crate::harness::submission_id(
        &app.submit(closed.id().as_uuid(), app.student(), "pull/makeup")
            .await
            .created(),
    );
    app.review(rejected, app.teacher(), "reject", "Не то")
        .await
        .ok();

    let after_rejection = progress(&app, closed.id().as_uuid()).await;
    assert_eq!(after_rejection["used_attempts"], json!(0));
    assert_eq!(
        after_rejection["remaining_attempts"],
        after_rejection["allowed_attempts"]
    );
    assert_eq!(after_rejection["has_accepted_submission"], json!(false));

    let accepted = crate::harness::submission_id(
        &app.submit(closed.id().as_uuid(), app.student(), "pull/makeup-2")
            .await
            .created(),
    );
    app.review(accepted, app.teacher(), "accept", "Досдано")
        .await
        .ok();

    let after_acceptance = progress(&app, closed.id().as_uuid()).await;
    assert_eq!(after_acceptance["used_attempts"], json!(0));
    assert_eq!(after_acceptance["has_accepted_submission"], json!(true));
}

async fn progress(app: &TestApp, assignment: Uuid) -> Value {
    app.student_assignment(assignment, app.student()).await.ok()["progress"].clone()
}
