use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::harness::TestApp;

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
async fn a_draft_assignment_stays_out_of_the_student_list() {
    let app = TestApp::start().await;
    app.add_draft_assignment().await;

    let body = app.student_assignments(app.student()).await.ok();

    only(&body);
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

/// Ради чего нужна карточка: комментарий ревьюера студенту больше негде прочитать.
#[tokio::test]
async fn the_card_carries_every_attempt_with_its_review() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    app.review(first, app.teacher(), "reject", "Add tests.")
        .await
        .ok();
    app.queued_submission(app.student(), "pull/2").await;

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    let submissions = body["submissions"].as_array().expect("a json array");
    assert_eq!(submissions.len(), 2);
    assert_eq!(submissions[0]["attempt_number"], json!(1));
    assert_eq!(submissions[0]["review_comment"], json!("Add tests."));
    assert_eq!(submissions[1]["attempt_number"], json!(2));
    assert_eq!(submissions[1]["status"], json!("pending_review"));
}

#[tokio::test]
async fn the_card_of_an_untouched_assignment_has_no_submissions() {
    let app = TestApp::start().await;

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await
        .ok();

    assert_eq!(body["submissions"], json!([]));
    assert_eq!(
        body["assignment"]["body"],
        json!("Send a pull request link.")
    );
}

/// Пересдача: задание сняли, но свои попытки и комментарии студент читать не перестаёт.
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

/// 404, а не 403: о существовании черновика студент знать не должен.
#[tokio::test]
async fn a_draft_card_does_not_exist_for_a_student() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;

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
        .error(
            StatusCode::FORBIDDEN,
            "student_does_not_belong_to_assignment_group",
        );
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
