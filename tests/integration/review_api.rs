use axum::http::StatusCode;
use chrono::Utc;
use hw_bot::app::{ChangeTerms, ErrorCode, ReviewSubmission};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, MakeupTerms, ReviewDecision, Submission, SubmissionId, SubmissionStatus,
};
use serde_json::json;
use uuid::Uuid;

use crate::harness::{TestApp, submission_id, wait_for_blocked_connection};

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
            Utc::now(),
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
            Utc::now(),
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
async fn an_earlier_sandbox_submission_stays_out_of_the_queue() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    app.submit(trial.id().as_uuid(), app.student(), "pull/trial")
        .await
        .created();
    let regular = app.queued_submission(app.student(), "pull/1").await;

    let body = app.take_next(app.teacher()).await.ok();
    assert_eq!(submission_id(&body["submission"]), regular);

    let body = app.review_queue_size(app.teacher()).await.ok();
    assert_eq!(body["pending"], json!(1));
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

    assert_eq!(
        stored(&app, submission).await.status(),
        SubmissionStatus::PendingReview
    );
}

#[tokio::test]
async fn a_review_that_loses_the_race_writes_neither_its_decision_nor_a_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let mut winner = app.pool.begin().await.expect("begin the winning review");
    let blocker: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *winner)
        .await
        .expect("read backend pid");
    sqlx::query(
        "update submissions
         set status = 'rejected', reviewed_by = $2, reviewed_at = now(), review_comment = 'Победитель'
         where id = $1",
    )
    .bind(submission)
    .bind(app.fixture.teacher.id().as_uuid())
    .execute(&mut *winner)
    .await
    .expect("write the winning review");

    let (loser, ()) = tokio::join!(
        app.review(submission, app.teacher(), "accept", "Проигравший"),
        async {
            wait_for_blocked_connection(&app.pool, blocker).await;
            winner.commit().await.expect("commit the winning review");
        },
    );

    loser.error(StatusCode::CONFLICT, "submission_already_reviewed");
    let saved = stored(&app, submission).await;
    assert_eq!(
        (
            saved.status(),
            saved.review_comment().map(|comment| comment.as_str())
        ),
        (SubmissionStatus::Rejected, Some("Победитель"))
    );
    let notifications: i64 = sqlx::query_scalar(
        "select count(*) from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .expect("count notifications");
    assert_eq!(notifications, 0);
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

async fn stored(app: &TestApp, submission: Uuid) -> Submission {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .expect("read submission")
        .expect("submission exists")
}

async fn pending_makeup(app: &TestApp) -> (Assignment, Uuid) {
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    let makeup = submission_id(
        &app.submit(closed.id().as_uuid(), app.student(), "pull/makeup")
            .await
            .created(),
    );

    (closed, makeup)
}

#[tokio::test]
async fn a_makeup_accepted_without_a_score_is_credited_by_its_terms() {
    let app = TestApp::start().await;
    let (_, makeup) = pending_makeup(&app).await;

    let reviewed = app
        .review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();

    assert_eq!(reviewed["kind"], "makeup");
    assert_eq!(reviewed["work_score"], 6);
    assert_eq!(reviewed["max_score"], 10);
    assert_eq!(reviewed["course_total_penalty"], 0);
    assert_eq!(reviewed["penalty_deducted_from"], json!(null));
}

#[tokio::test]
async fn a_makeup_review_reads_the_terms_in_force_when_it_is_saved() {
    let app = TestApp::start().await;
    let (closed, makeup) = pending_makeup(&app).await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: closed.id(),
            actor: app.teacher_actor(),
            deadline_at: closed.deadline_at(),
            late_policy: None,
            makeup: Some(MakeupTerms::NotOffered),
            expected_revision: 0,
        })
        .await
        .unwrap();

    let error = app
        .application
        .reviews
        .review_submission(ReviewSubmission {
            submission_id: SubmissionId::from_uuid(makeup),
            actor: app.teacher_actor(),
            decision: ReviewDecision::Accept,
            score: None,
            comment: "Досдано".to_owned(),
            reviewed_at: Utc::now(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert_eq!(
        stored(&app, makeup).await.status(),
        SubmissionStatus::PendingReview
    );
}

#[tokio::test]
async fn the_queue_and_its_counter_stay_empty_while_only_makeups_wait() {
    let app = TestApp::start().await;
    pending_makeup(&app).await;

    app.take_next(app.teacher()).await.no_content();
    assert_eq!(
        app.review_queue_size(app.teacher()).await.ok()["pending"],
        0
    );
}
