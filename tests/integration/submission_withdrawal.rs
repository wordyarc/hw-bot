use chrono::Utc;
use hw_bot::app::{
    Actor, AppError, ConfigureMakeup, ErrorCode, ReviewSandbox, ReviewSubmission, SubmitHomework,
    WithdrawSubmission,
};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, MakeupPool, Resubmission, ReviewDecision, Submission, SubmissionHistory,
    SubmissionId, SubmissionStatus,
};
use uuid::Uuid;

use crate::harness::{TestApp, submission_id, wait_for_blocked_connection};

async fn withdraw(app: &TestApp, submission: Uuid) -> Result<Submission, AppError> {
    app.application
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: app.student_actor(),
            withdrawn_at: Utc::now(),
        })
        .await
}

async fn find(app: &TestApp, submission: Uuid) -> Submission {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .expect("read submission")
        .expect("submission exists")
}

async fn reject(app: &TestApp, submission: Uuid) {
    app.application
        .reviews
        .review_submission(ReviewSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: app.teacher_actor(),
            decision: ReviewDecision::Reject,
            score: None,
            comment: "Не то".to_owned(),
            reviewed_at: Utc::now(),
            return_attempt: false,
        })
        .await
        .expect("the teacher rejects");
}

async fn history(app: &TestApp) -> SubmissionHistory {
    app.application
        .assignments
        .find_for_student(app.student_actor(), app.fixture.assignment.id())
        .await
        .expect("the assignment is visible")
        .assignment
        .history
}

async fn submit_makeup(app: &TestApp, assignment: &Assignment) -> Uuid {
    app.application
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id: assignment.id(),
            actor: app.student_actor(),
            content: "pull/makeup".to_owned(),
            submitted_at: Utc::now(),
            allow_makeup: true,
        })
        .await
        .expect("the makeup is accepted")
        .submission
        .id()
        .as_uuid()
}

async fn student_pool(app: &TestApp) -> MakeupPool {
    app.application
        .courses
        .student_makeup(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
            Utc::now(),
        )
        .await
        .expect("the pool is readable")
        .expect("makeup is enabled")
}

async fn disable_makeup(app: &TestApp) {
    let enabled = app
        .application
        .courses
        .active_makeup(app.teacher_actor(), app.fixture.course.id(), Utc::now())
        .await
        .expect("the makeup is readable");
    app.application
        .courses
        .configure_makeup(ConfigureMakeup {
            course_id: app.fixture.course.id(),
            actor: app.teacher_actor(),
            expected: enabled,
            settings: None,
            decisions: Vec::new(),
            now: Utc::now(),
        })
        .await
        .expect("the teacher disables makeup");
}

#[tokio::test]
async fn a_withdrawal_marks_the_submission_withdrawn_without_spending_an_attempt() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    withdraw(&app, submission).await.unwrap();

    let stored = find(&app, submission).await;
    assert_eq!(stored.status(), SubmissionStatus::Withdrawn);
    assert!(stored.withdrawn_at().is_some());
    let history = history(&app).await;
    assert_eq!(history.used_attempts(), 0);
    assert!(!history.has_pending_submission());
}

#[tokio::test]
async fn a_student_does_not_withdraw_another_students_submission() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let other = app.add_student("Other student").await;

    let error = app
        .application
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: Actor::new(other.id()),
            withdrawn_at: Utc::now(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(find(&app, submission).await.is_pending_review());
}

#[tokio::test]
async fn a_submission_after_a_withdrawal_takes_the_next_number() {
    let app = TestApp::start().await;
    let withdrawn = app.queued_submission(app.student(), "pull/1").await;
    withdraw(&app, withdrawn).await.unwrap();

    let next = app.queued_submission(app.student(), "pull/2").await;

    assert_eq!(find(&app, withdrawn).await.attempt_number(), 1);
    assert_eq!(find(&app, next).await.attempt_number(), 2);
    assert_eq!(history(&app).await.used_attempts(), 0);
}

#[tokio::test]
async fn a_withdrawal_on_the_last_attempt_keeps_the_attempt() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    reject(&app, first).await;
    let last = app.queued_submission(app.student(), "pull/2").await;

    withdraw(&app, last).await.unwrap();

    let history = history(&app).await;
    assert_eq!(history.remaining_attempts_for(&app.fixture.assignment), 1);
    app.queued_submission(app.student(), "pull/3").await;
}

#[tokio::test]
async fn a_withdrawn_makeup_returns_its_attempt_to_the_pool() {
    let app = TestApp::start().await;
    let work = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(1).await;
    let makeup = submit_makeup(&app, &work).await;
    assert_eq!(student_pool(&app).await.balance.remaining(), 0);

    withdraw(&app, makeup).await.unwrap();

    assert_eq!(student_pool(&app).await.balance.remaining(), 1);
}

#[tokio::test]
async fn a_makeup_of_an_earlier_enabling_returns_nothing_to_the_new_pool() {
    let app = TestApp::start().await;
    let earlier = app.add_closed_assignment("Earlier", 10, 6).await;
    let later = app.add_closed_assignment("Later", 10, 6).await;
    app.enable_makeup(2).await;
    let old_makeup = submit_makeup(&app, &earlier).await;
    disable_makeup(&app).await;
    app.enable_makeup(2).await;
    submit_makeup(&app, &later).await;

    let pending = app
        .application
        .submissions
        .find_for_withdrawal(app.student_actor(), SubmissionId::from_uuid(old_makeup))
        .await
        .unwrap();

    let Resubmission::Makeup(offer) = pending.resubmission_at(Utc::now()) else {
        panic!("the earlier work can be sent again as a makeup");
    };
    assert_eq!(offer.balance.remaining(), 1);
}

#[tokio::test]
async fn a_withdrawn_submission_leaves_the_queue_and_its_counter() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    withdraw(&app, submission).await.unwrap();

    let reviews = &app.application.reviews;
    assert_eq!(
        reviews.queue_size(app.teacher_actor(), None).await.unwrap(),
        0
    );
    assert!(
        reviews
            .take_next_from_queue(app.teacher_actor(), None, Utc::now())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn opening_a_withdrawn_submission_for_review_reports_the_withdrawal() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    withdraw(&app, submission).await.unwrap();

    let error = app
        .application
        .reviews
        .find_for_review(
            app.teacher_actor(),
            SubmissionId::from_uuid(submission),
            Utc::now(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::SubmissionAlreadyWithdrawn);
}

#[tokio::test]
async fn the_review_card_omits_withdrawn_previous_attempts() {
    let app = TestApp::start().await;
    let rejected = app.queued_submission(app.student(), "pull/1").await;
    reject(&app, rejected).await;
    let withdrawn = app.queued_submission(app.student(), "pull/2").await;
    withdraw(&app, withdrawn).await.unwrap();
    let current = app.queued_submission(app.student(), "pull/3").await;

    let item = app
        .application
        .reviews
        .find_for_review(
            app.teacher_actor(),
            SubmissionId::from_uuid(current),
            Utc::now(),
        )
        .await
        .unwrap();

    let previous: Vec<Uuid> = item
        .previous_attempts
        .iter()
        .map(|attempt| attempt.id().as_uuid())
        .collect();
    assert_eq!(previous, vec![rejected]);
}

#[tokio::test]
async fn a_bulk_sandbox_decision_skips_a_withdrawn_submission() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let submission = submission_id(
        &app.submit(trial.id().as_uuid(), app.student(), "pull/trial")
            .await
            .created(),
    );
    withdraw(&app, submission).await.unwrap();

    let reviewed = app
        .application
        .reviews
        .review_sandbox(ReviewSandbox {
            actor: app.teacher_actor(),
            decision: ReviewDecision::Reject,
            cutoff: Utc::now(),
            comment: "Учебная сдача".to_owned(),
            reviewed_at: Utc::now(),
        })
        .await
        .unwrap();

    assert_eq!(reviewed, 0);
    assert_eq!(
        find(&app, submission).await.status(),
        SubmissionStatus::Withdrawn
    );
}

async fn blocking_transaction(app: &TestApp) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut tx = app.pool.begin().await.expect("begin a concurrent writer");
    let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .expect("read backend pid");

    (tx, pid)
}

#[tokio::test]
async fn a_withdrawal_waiting_for_a_decision_reports_the_review() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let (mut reviewer, blocker) = blocking_transaction(&app).await;
    sqlx::query(
        "update submissions
         set status = 'rejected', reviewed_by = $2, reviewed_at = now(), review_comment = 'Нет'
         where id = $1",
    )
    .bind(submission)
    .bind(app.fixture.teacher.id().as_uuid())
    .execute(&mut *reviewer)
    .await
    .expect("decide concurrently");

    let (withdrawn, ()) = tokio::join!(withdraw(&app, submission), async {
        wait_for_blocked_connection(&app.pool, blocker).await;
        reviewer.commit().await.expect("commit the decision");
    });

    assert_eq!(
        withdrawn.unwrap_err().code(),
        ErrorCode::SubmissionAlreadyReviewed
    );
    assert_eq!(
        find(&app, submission).await.status(),
        SubmissionStatus::Rejected
    );
}

#[tokio::test]
async fn a_decision_waiting_for_a_withdrawal_reports_it_and_writes_nothing() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let (mut student, blocker) = blocking_transaction(&app).await;
    sqlx::query("update submissions set status = 'withdrawn', withdrawn_at = now() where id = $1")
        .bind(submission)
        .execute(&mut *student)
        .await
        .expect("withdraw concurrently");

    let review = app.application.reviews.review_submission(ReviewSubmission {
        submission_id: SubmissionId::from_uuid(submission),
        actor: app.teacher_actor(),
        decision: ReviewDecision::Reject,
        score: None,
        comment: "На доработку".to_owned(),
        reviewed_at: Utc::now(),
        return_attempt: true,
    });
    let (reviewed, ()) = tokio::join!(review, async {
        wait_for_blocked_connection(&app.pool, blocker).await;
        student.commit().await.expect("commit the withdrawal");
    });

    assert_eq!(
        reviewed.unwrap_err().code(),
        ErrorCode::SubmissionAlreadyWithdrawn
    );
    let notifications: i64 = sqlx::query_scalar("select count(*) from review_notification_outbox")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(notifications, 0);
    assert_eq!(history(&app).await.extra_attempts(), 0);
}

#[tokio::test]
async fn a_bulk_decision_waiting_for_a_withdrawal_leaves_it_withdrawn() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let submission = submission_id(
        &app.submit(trial.id().as_uuid(), app.student(), "pull/trial")
            .await
            .created(),
    );
    let (mut student, blocker) = blocking_transaction(&app).await;
    sqlx::query("update submissions set status = 'withdrawn', withdrawn_at = now() where id = $1")
        .bind(submission)
        .execute(&mut *student)
        .await
        .expect("withdraw concurrently");

    let bulk = app.application.reviews.review_sandbox(ReviewSandbox {
        actor: app.teacher_actor(),
        decision: ReviewDecision::Reject,
        cutoff: Utc::now(),
        comment: "Учебная сдача".to_owned(),
        reviewed_at: Utc::now(),
    });
    let (reviewed, ()) = tokio::join!(bulk, async {
        wait_for_blocked_connection(&app.pool, blocker).await;
        student.commit().await.expect("commit the withdrawal");
    });

    assert_eq!(reviewed.unwrap(), 0);
    assert_eq!(
        find(&app, submission).await.status(),
        SubmissionStatus::Withdrawn
    );
}
