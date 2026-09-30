use axum::http::StatusCode;
use serde_json::json;
use uuid::Uuid;

use crate::harness::{TestApp, submission_id};

#[tokio::test]
async fn reloading_a_review_checks_access_to_its_course() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let outsider = app.add_outsider_teacher("Outsider").await;
    let error = app
        .application
        .reviews
        .find_for_review(
            hw_bot::app::Actor::new(outsider.id()),
            hw_bot::domain::SubmissionId::from_uuid(submission),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), hw_bot::app::ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn reloading_a_completed_review_does_not_restart_it() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Принято")
        .await
        .ok();
    let error = app
        .application
        .reviews
        .find_for_review(
            hw_bot::app::Actor::new(app.fixture.teacher.id()),
            hw_bot::domain::SubmissionId::from_uuid(submission),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.code(),
        hw_bot::app::ErrorCode::SubmissionAlreadyReviewed
    );
}

#[tokio::test]
async fn empty_queue_returns_no_content() {
    let app = TestApp::start().await;

    app.take_next(app.teacher()).await.no_content();
}

#[tokio::test]
async fn queue_is_served_in_submission_order() {
    let app = TestApp::start().await;
    let second_student = app.add_student("Second student").await;

    let first = app.queued_submission(app.student(), "pull/1").await;
    let second = app
        .queued_submission(second_student.telegram_user_id().as_i64(), "pull/2")
        .await;

    let body = app.take_next(app.teacher()).await.ok();
    assert_eq!(submission_id(&body["submission"]), first);

    app.review(first, app.teacher(), "accept", "ok").await.ok();

    let body = app.take_next(app.teacher()).await.ok();
    assert_eq!(
        submission_id(&body["submission"]),
        second,
        "the reviewed submission leaves the queue"
    );
}

#[tokio::test]
async fn queue_item_carries_assignment_and_student_context() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;

    let body = app.take_next(app.teacher()).await.ok();

    assert_eq!(body["submission"]["status"], json!("pending_review"));
    assert_eq!(body["assignment"]["id"], json!(app.assignment()));
    assert_eq!(body["assignment"]["title"], json!("Rust ownership"));
    assert_eq!(body["assignment"]["max_attempts"], json!(2));
    assert_eq!(body["student"]["display_name"], json!("Student"));
    assert_eq!(body["student"]["telegram_user_id"], json!(app.student()));
}

#[tokio::test]
async fn student_cannot_take_from_the_queue() {
    let app = TestApp::start().await;

    app.take_next(app.student())
        .await
        .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn teacher_sees_only_submissions_of_their_own_courses() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;

    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    app.take_next(outsider.telegram_user_id().as_i64())
        .await
        .no_content();
}

#[tokio::test]
async fn second_course_teacher_shares_the_queue() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let reviewer = app.add_course_teacher("Second reviewer").await;

    let body = app
        .take_next(reviewer.telegram_user_id().as_i64())
        .await
        .ok();

    assert_eq!(submission_id(&body["submission"]), submission);
}

#[tokio::test]
async fn admin_reviews_every_course() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let foreign_student = app
        .add_student_to_group("Foreign student", &foreign.group)
        .await;
    let foreign_assignment = app
        .add_published_assignment(&foreign.course, &foreign.teacher, "Foreign assignment")
        .await;

    app.submit(
        foreign_assignment.id().as_uuid(),
        foreign_student.telegram_user_id().as_i64(),
        "pull/1",
    )
    .await
    .created();

    let admin = app.add_admin().await;
    let body = app.take_next(admin.telegram_user_id().as_i64()).await.ok();

    assert_eq!(
        body["assignment"]["id"],
        json!(foreign_assignment.id().as_uuid()),
        "an admin reviews courses they do not teach"
    );
}

#[tokio::test]
async fn queue_does_not_mix_courses() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let foreign_student = app
        .add_student_to_group("Foreign student", &foreign.group)
        .await;
    let foreign_assignment = app
        .add_published_assignment(&foreign.course, &foreign.teacher, "Foreign assignment")
        .await;

    let own = app.queued_submission(app.student(), "pull/1").await;
    app.submit(
        foreign_assignment.id().as_uuid(),
        foreign_student.telegram_user_id().as_i64(),
        "pull/2",
    )
    .await
    .created();

    let body = app.take_next(app.teacher()).await.ok();
    assert_eq!(submission_id(&body["submission"]), own);

    let body = app
        .take_next(foreign.teacher.telegram_user_id().as_i64())
        .await
        .ok();
    assert_eq!(
        body["assignment"]["id"],
        json!(foreign_assignment.id().as_uuid())
    );
}

#[tokio::test]
async fn outsider_teacher_cannot_review() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let outsider = app.add_outsider_teacher("Outsider teacher").await;

    app.review(
        submission,
        outsider.telegram_user_id().as_i64(),
        "reject",
        "not mine",
    )
    .await
    .error(StatusCode::FORBIDDEN, "permission_denied");
}

#[tokio::test]
async fn review_comment_is_mandatory_for_both_decisions() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    for decision in ["accept", "reject"] {
        app.review(submission, app.teacher(), decision, "   ")
            .await
            .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input");
    }
}

#[tokio::test]
async fn oversized_review_comment_is_rejected() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    app.review(submission, app.teacher(), "reject", &"я".repeat(4097))
        .await
        .error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_input");
}

#[tokio::test]
async fn reviewing_unknown_submission_is_not_found() {
    let app = TestApp::start().await;

    app.review(Uuid::now_v7(), app.teacher(), "accept", "good")
        .await
        .not_found();
}

#[tokio::test]
async fn rejection_is_stored_with_a_trimmed_comment() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    let body = app
        .review(
            submission,
            app.teacher(),
            "reject",
            "  Add tests, then resubmit.  ",
        )
        .await
        .ok();

    assert_eq!(body["status"], json!("rejected"));
    assert_eq!(
        body["reviewed_by"],
        json!(app.fixture.teacher.id().as_uuid())
    );
    assert_eq!(body["review_comment"], json!("Add tests, then resubmit."));
    assert!(body["reviewed_at"].as_str().is_some());
}

#[tokio::test]
async fn completed_review_enqueues_notification_for_the_student() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    app.review(submission, app.teacher(), "accept", "  95/100  ")
        .await
        .ok();

    let notification = sqlx::query!(
        r#"
        select
            recipient_telegram_user_id,
            subject_name,
            assignment_title,
            outcome,
            review_comment
        from review_notification_outbox
        where submission_id = $1
        "#,
        submission,
    )
    .fetch_one(&app.pool)
    .await
    .expect("notification is enqueued");

    assert_eq!(notification.recipient_telegram_user_id, app.student());
    assert_eq!(notification.subject_name, "Backend development");
    assert_eq!(notification.assignment_title, "Rust ownership");
    assert_eq!(notification.outcome, "accepted");
    assert_eq!(notification.review_comment, "95/100");
}

#[tokio::test]
async fn outbox_failure_rolls_back_the_review() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    sqlx::query(
        r#"
        alter table review_notification_outbox
        add constraint reject_notifications check (false) not valid
        "#,
    )
    .execute(&app.pool)
    .await
    .expect("add rejecting constraint");

    app.review(submission, app.teacher(), "accept", "good")
        .await
        .expect(StatusCode::INTERNAL_SERVER_ERROR);

    let status: String = sqlx::query_scalar("select status from submissions where id = $1")
        .bind(submission)
        .fetch_one(&app.pool)
        .await
        .expect("load submission status");
    assert_eq!(status, "pending_review");
}

#[tokio::test]
async fn racing_reviews_enqueue_one_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    let (first, second) = tokio::join!(
        app.review(submission, app.teacher(), "accept", "good"),
        app.review(submission, app.teacher(), "reject", "bad"),
    );

    let (winner, loser) = if first.status == StatusCode::OK {
        (first, second)
    } else {
        (second, first)
    };
    winner.ok();
    loser.error(StatusCode::CONFLICT, "submission_already_reviewed");

    let count: i64 = sqlx::query_scalar(
        "select count(*) from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .expect("count notifications");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn a_finished_review_is_final() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "reject", "Add tests.")
        .await
        .ok();

    app.review(submission, app.teacher(), "accept", "changed my mind")
        .await
        .error(StatusCode::CONFLICT, "submission_already_reviewed");
}

#[tokio::test]
async fn rejected_submission_spends_an_attempt() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    app.review(first, app.teacher(), "reject", "Add tests.")
        .await
        .ok();

    let body = app
        .submit(app.assignment(), app.student(), "pull/2")
        .await
        .created();

    assert_eq!(body["attempt_number"], json!(2));
}

#[tokio::test]
async fn accepted_submission_cannot_be_redone_while_attempts_remain() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    let body = app
        .review(submission, app.teacher(), "accept", "95/100")
        .await
        .ok();
    assert_eq!(body["status"], json!("accepted"));
    assert_eq!(body["review_comment"], json!("95/100"));

    app.submit(app.assignment(), app.student(), "pull/2")
        .await
        .error(StatusCode::CONFLICT, "submission_already_accepted");
}

#[tokio::test]
async fn attempts_run_out_after_the_last_rejection() {
    let app = TestApp::start().await;

    // Третья попытка превышает заданный фикстурой max_attempts = 2.
    for attempt in 1..=2 {
        let submission = app
            .queued_submission(app.student(), &format!("pull/{attempt}"))
            .await;
        app.review(submission, app.teacher(), "reject", "Again, please.")
            .await
            .ok();
    }

    app.submit(app.assignment(), app.student(), "pull/3")
        .await
        .error(StatusCode::CONFLICT, "attempts_exhausted");
}
