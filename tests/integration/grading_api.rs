use axum::http::StatusCode;
use chrono::{Duration, Utc};
use hw_bot::db;
use hw_bot::domain::Grading;
use serde_json::json;

use crate::harness::{TestApp, submission_id};

async fn scored_submission(app: &TestApp, max_score: u16) -> uuid::Uuid {
    let assignment = app.add_scored_assignment(max_score).await;
    let body = app
        .submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await;

    submission_id(&body.created())
}

#[tokio::test]
async fn accepting_with_a_score_stores_the_whole_arithmetic() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    let body = app
        .review_with_score(submission, app.teacher(), "accept", 4, "Хорошо")
        .await
        .ok();

    assert_eq!(body["status"], json!("accepted"));
    assert_eq!(body["raw_score"], json!(4));
    assert_eq!(body["max_score"], json!(5));
    assert_eq!(body["late_periods"], json!(0));
    assert_eq!(body["penalty"], json!(0));
    assert_eq!(body["final_score"], json!(4));
}

#[tokio::test]
async fn a_submission_awaiting_review_has_no_grade() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;

    let body = app
        .submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .created();

    assert_eq!(body["raw_score"], json!(null));
    assert_eq!(body["max_score"], json!(null));
    assert_eq!(body["final_score"], json!(null));
}

#[tokio::test]
async fn rejecting_a_scored_submission_stores_no_grade() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    let body = app
        .review(submission, app.teacher(), "reject", "Нет тестов")
        .await
        .ok();

    assert_eq!(body["status"], json!("rejected"));
    assert_eq!(body["raw_score"], json!(null));
    assert_eq!(body["max_score"], json!(null));
    assert_eq!(body["final_score"], json!(null));
}

#[tokio::test]
async fn the_notification_carries_the_score_of_the_review() {
    let app = TestApp::start().await;
    let submission = scored_submission(&app, 5).await;

    app.review_with_score(submission, app.teacher(), "accept", 4, "Хорошо")
        .await
        .ok();

    let notification = sqlx::query!(
        "select final_score, max_score from review_notification_outbox where submission_id = $1",
        submission,
    )
    .fetch_one(&app.pool)
    .await
    .expect("notification is enqueued");

    assert_eq!(notification.final_score, Some(4));
    assert_eq!(notification.max_score, Some(5));
}

#[tokio::test]
async fn a_review_freezes_the_scale_after_the_assignment_becomes_pass_fail() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(submission, app.teacher(), "accept", 4, "Хорошо")
        .await
        .ok();

    app.change_grading(assignment.id().as_uuid(), app.teacher(), None)
        .await
        .ok();

    let details = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();
    assert_eq!(details["submissions"][0]["max_score"], json!(5));

    let gradebook = app
        .get_as(
            &format!("/me/courses/{}/gradebook", app.course()),
            app.teacher(),
        )
        .await
        .ok();
    assert_eq!(gradebook["students"][0]["grades"][0]["max_score"], json!(5));
    assert_eq!(gradebook["students"][0]["max_total"], json!(5));
}

#[tokio::test]
async fn a_pass_fail_notification_carries_no_score() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    let notification = sqlx::query!(
        "select final_score, max_score from review_notification_outbox where submission_id = $1",
        submission,
    )
    .fetch_one(&app.pool)
    .await
    .expect("notification is enqueued");

    assert_eq!(notification.final_score, None);
    assert_eq!(notification.max_score, None);
}

#[tokio::test]
async fn an_assignment_without_a_scale_stays_pass_fail() {
    let app = TestApp::start().await;

    let body = app
        .student_assignment(app.assignment(), app.student())
        .await;

    assert_eq!(body.ok()["assignment"]["max_score"], json!(null));
}

#[tokio::test]
async fn a_scale_can_be_set_after_creation() {
    let app = TestApp::start().await;
    let draft = app.drafted_assignment(app.teacher()).await;

    let body = app
        .change_grading(draft, app.teacher(), Some(10))
        .await
        .ok();

    assert_eq!(body["max_score"], json!(10));
}

#[tokio::test]
async fn a_scale_of_an_archived_assignment_is_frozen() {
    let app = TestApp::start().await;
    let draft = app.drafted_assignment(app.teacher()).await;
    app.archive_assignment(draft, app.teacher()).await.ok();

    app.change_grading(draft, app.teacher(), Some(10))
        .await
        .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn a_scale_change_advances_the_revision() {
    let app = TestApp::start().await;
    let draft = app.drafted_assignment(app.teacher()).await;

    app.change_grading(draft, app.teacher(), Some(10))
        .await
        .ok();

    let revision: i64 = sqlx::query_scalar("select revision from assignments where id = $1")
        .bind(draft)
        .fetch_one(&app.pool)
        .await
        .expect("read revision");
    assert_eq!(revision, 1);
}

#[tokio::test]
async fn a_scale_change_on_a_stale_revision_loses() {
    let app = TestApp::start().await;
    let mut assignment = app.add_draft_assignment().await;
    app.change_deadline(
        assignment.id().as_uuid(),
        app.teacher(),
        Utc::now() + Duration::days(14),
    )
    .await
    .ok();

    assignment
        .change_grading(
            &app.fixture.course,
            &app.fixture.teacher,
            Grading::scored(10).unwrap(),
            None,
        )
        .expect("teacher may change the scale");

    let result = db::assignments::update(&app.pool, &assignment, 0).await;

    assert!(
        matches!(result, Err(db::DbError::Conflict { .. })),
        "a stale revision must not overwrite the winner, got {result:?}"
    );
}

#[tokio::test]
async fn a_scale_given_at_creation_reaches_the_response() {
    let app = TestApp::start().await;

    let body = app
        .create_assignment_raw(
            app.course(),
            app.teacher(),
            json!({
                "title": "Rust lifetimes",
                "body": "Send a pull request link.",
                "max_attempts": 2,
                "max_score": 10,
                "deadline_at": Utc::now() + Duration::days(7),
            }),
        )
        .await
        .created();

    assert_eq!(body["max_score"], json!(10));
}

#[tokio::test]
async fn a_late_submission_before_the_horizon_is_accepted_with_a_penalty() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;

    let submitted = app
        .submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .created();
    let body = app
        .review_with_score(submission_id(&submitted), app.teacher(), "accept", 5, "Ок")
        .await
        .ok();

    assert_eq!(body["late_periods"], json!(1));
    assert_eq!(body["penalty"], json!(1));
    assert_eq!(body["final_score"], json!(4));
}

#[tokio::test]
async fn a_submission_past_the_horizon_is_closed() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(29, Some((7, 1, 4))).await;

    app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .error(StatusCode::CONFLICT, "submission_closed");
}

#[tokio::test]
async fn an_assignment_without_a_late_policy_still_reports_the_deadline() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, None).await;

    app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
        .await
        .error(StatusCode::CONFLICT, "deadline_exceeded");
}

#[tokio::test]
async fn moving_the_deadline_moves_the_horizon() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(29, Some((7, 1, 4))).await;
    let id = assignment.id().as_uuid();

    app.change_deadline(id, app.teacher(), Utc::now() - Duration::days(1))
        .await
        .ok();

    app.submit(id, app.student(), "pull/1").await.created();
}

#[tokio::test]
async fn an_incomplete_late_policy_is_rejected() {
    let app = TestApp::start().await;

    app.create_assignment_raw(
        app.course(),
        app.teacher(),
        json!({
            "title": "Half a policy",
            "body": "Send a pull request link.",
            "max_attempts": 2,
            "max_score": 5,
            "late_period_days": 7,
            "deadline_at": Utc::now() + Duration::days(7),
        }),
    )
    .await
    .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input");
}

#[tokio::test]
async fn the_horizon_reaches_the_assignment_response() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;

    let body = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();

    let card = &body["assignment"];
    assert_eq!(card["late_period_days"], json!(7));
    assert_eq!(card["late_penalty"], json!(1));
    assert_eq!(card["late_max_periods"], json!(4));
    assert_eq!(
        card["closes_at"].as_str().unwrap(),
        (assignment.deadline_at() + Duration::days(28))
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
    );
}

async fn gradebook(app: &TestApp, telegram_id: i64) -> crate::harness::TestResponse {
    app.get_as(
        &format!("/me/courses/{}/gradebook", app.course()),
        telegram_id,
    )
    .await
}

#[tokio::test]
async fn a_student_without_submissions_is_still_in_the_gradebook() {
    let app = TestApp::start().await;
    app.add_scored_assignment(5).await;

    let body = gradebook(&app, app.teacher()).await.ok();

    let students = body["students"].as_array().unwrap();
    assert_eq!(students.len(), 1);
    assert_eq!(students[0]["total"], json!(0));
    assert_eq!(students[0]["max_total"], json!(5));
    assert_eq!(students[0]["grades"][0]["final_score"], json!(null));
}

#[tokio::test]
async fn the_gradebook_sums_only_reviewed_scores() {
    let app = TestApp::start().await;
    let first = app.add_scored_assignment(5).await;
    let second = app.add_scored_assignment(10).await;

    let reviewed = submission_id(
        &app.submit(first.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(reviewed, app.teacher(), "accept", 4, "Ок")
        .await
        .ok();
    app.submit(second.id().as_uuid(), app.student(), "pull/2")
        .await
        .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["total"], json!(4), "сдача на проверке не считается");
    assert_eq!(student["max_total"], json!(15));
}

#[tokio::test]
async fn pass_fail_assignments_stay_out_of_the_gradebook() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    let body = gradebook(&app, app.teacher()).await.ok();

    let student = &body["students"][0];
    assert_eq!(student["grades"].as_array().unwrap().len(), 0);
    assert_eq!(student["max_total"], json!(0));
}

#[tokio::test]
async fn a_gradebook_of_a_foreign_course_is_denied() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    gradebook(&app, outsider.telegram_user_id().as_i64())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_student_cannot_read_the_gradebook() {
    let app = TestApp::start().await;

    gradebook(&app, app.student())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn a_draft_assignment_is_not_in_the_gradebook_yet() {
    let app = TestApp::start().await;
    app.create_assignment_raw(
        app.course(),
        app.teacher(),
        json!({
            "title": "Draft with a scale",
            "body": "Send a pull request link.",
            "max_attempts": 2,
            "max_score": 5,
            "deadline_at": Utc::now() + Duration::days(7),
        }),
    )
    .await
    .created();

    let body = gradebook(&app, app.teacher()).await.ok();

    assert_eq!(body["students"][0]["grades"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn moving_the_deadline_does_not_change_a_grade_that_was_already_given() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let reviewed = app
        .review_with_score(submission, app.teacher(), "accept", 5, "Ок")
        .await
        .ok();
    assert_eq!(reviewed["final_score"], json!(4));

    app.change_deadline(
        assignment.id().as_uuid(),
        app.teacher(),
        Utc::now() + Duration::days(7),
    )
    .await
    .ok();

    let body = app
        .student_assignment(assignment.id().as_uuid(), app.student())
        .await
        .ok();

    let stored = &body["submissions"][0];
    assert_eq!(stored["late_periods"], json!(1));
    assert_eq!(stored["penalty"], json!(1));
    assert_eq!(stored["final_score"], json!(4));
}
