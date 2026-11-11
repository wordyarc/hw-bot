use chrono::Utc;
use hw_bot::app::{ErrorCode, RemainingAttempts, ReviewSubmission, SubjectQueue};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, ReviewDecision, SubjectId, SubmissionId, SubmissionStatus, TeachingCourse, User,
};
use hw_bot::test_support::subject;
use uuid::Uuid;

use crate::harness::{
    TestApp, insert_assignment, submission_id, wait_for_blocked_connection, wall_clock_assignment,
};

async fn submit(app: &TestApp, assignment: &Assignment, student: &User, content: &str) -> Uuid {
    submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            student.telegram_user_id().as_i64(),
            content,
        )
        .await
        .created(),
    )
}

async fn second_subject(app: &TestApp) -> (TeachingCourse, Assignment) {
    let course = app.add_subject_course("Frontend").await;
    let assignment = app
        .add_published_assignment(&course, &app.fixture.teacher, "Layout")
        .await;

    (course, assignment)
}

async fn add_makeup_and_sandbox(app: &TestApp) {
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    submit(app, &closed, &app.fixture.student, "pull/makeup").await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    submit(app, &trial, &app.fixture.student, "pull/trial").await;
}

async fn next_id(app: &TestApp, subject: Option<SubjectId>) -> Option<Uuid> {
    app.application
        .reviews
        .take_next_from_queue(app.teacher_actor(), subject, Utc::now())
        .await
        .expect("the queue is readable")
        .map(|item| item.submission.id().as_uuid())
}

async fn size(app: &TestApp, subject: Option<SubjectId>) -> u32 {
    app.application
        .reviews
        .queue_size(app.teacher_actor(), subject)
        .await
        .expect("the queue is countable")
}

async fn by_subject(app: &TestApp) -> Vec<SubjectQueue> {
    app.application
        .reviews
        .queue_by_subject(app.teacher_actor())
        .await
        .expect("the queue is countable by subject")
}

async fn list_ids(app: &TestApp) -> Vec<Uuid> {
    app.application
        .reviews
        .queue_page(app.teacher_actor(), Default::default())
        .await
        .expect("the queue list is readable")
        .items
        .iter()
        .map(|item| item.submission_id.as_uuid())
        .collect()
}

#[tokio::test]
async fn a_subject_queue_hands_out_the_earliest_work_of_that_subject() {
    let app = TestApp::start().await;
    let (_, layout) = second_subject(&app).await;
    submit(&app, &layout, &app.fixture.student, "pull/layout").await;
    let backend = app.queued_submission(app.student(), "pull/backend").await;

    assert_eq!(
        next_id(&app, Some(app.fixture.subject.id())).await,
        Some(backend)
    );
}

#[tokio::test]
async fn the_subject_counter_skips_makeups_and_sandbox() {
    let app = TestApp::start().await;
    add_makeup_and_sandbox(&app).await;
    app.queued_submission(app.student(), "pull/backend").await;

    assert_eq!(size(&app, Some(app.fixture.subject.id())).await, 1);
}

#[tokio::test]
async fn the_subject_counter_matches_what_its_queue_hands_out() {
    let app = TestApp::start().await;
    let (course, layout) = second_subject(&app).await;
    submit(&app, &layout, &app.fixture.student, "pull/layout").await;
    app.queued_submission(app.student(), "pull/backend").await;

    assert_eq!(size(&app, Some(course.subject_id())).await, 1);
    assert!(next_id(&app, Some(course.subject_id())).await.is_some());
}

#[tokio::test]
async fn the_counts_by_subject_add_up_to_the_main_queue() {
    let app = TestApp::start().await;
    let (course, layout) = second_subject(&app).await;
    submit(&app, &layout, &app.fixture.student, "pull/layout").await;
    let student = app.add_student("Second").await;
    submit(&app, &layout, &student, "pull/layout-2").await;
    app.queued_submission(app.student(), "pull/backend").await;
    add_makeup_and_sandbox(&app).await;

    let counts = by_subject(&app).await;

    let pending: Vec<_> = counts
        .iter()
        .map(|queue| (queue.subject_id, queue.pending))
        .collect();
    assert_eq!(pending.len(), 2, "{counts:?}");
    assert!(pending.contains(&(course.subject_id(), 2)), "{counts:?}");
    assert!(
        pending.contains(&(app.fixture.subject.id(), 1)),
        "{counts:?}"
    );
    assert_eq!(
        counts.iter().map(|queue| queue.pending).sum::<u32>(),
        size(&app, None).await
    );
}

#[tokio::test]
async fn a_subject_without_pending_work_is_absent_from_the_counts() {
    let app = TestApp::start().await;
    second_subject(&app).await;
    app.queued_submission(app.student(), "pull/backend").await;

    let subjects: Vec<_> = by_subject(&app)
        .await
        .into_iter()
        .map(|queue| queue.subject_id)
        .collect();

    assert_eq!(subjects, [app.fixture.subject.id()]);
}

#[tokio::test]
async fn a_foreign_subject_hands_out_nothing() {
    let app = TestApp::start().await;
    let android = subject("Android");
    db::academic::insert_subject(&app.pool, &android)
        .await
        .expect("insert subject");
    let foreign = app.add_foreign_course_on(&android).await;
    let student = app.add_student_to_group("Foreign", &foreign.group).await;
    let work = app
        .add_published_assignment(&foreign.course, &foreign.teacher, "Foreign")
        .await;
    submit(&app, &work, &student, "pull/foreign").await;

    assert_eq!(next_id(&app, Some(android.id())).await, None);
    assert_eq!(size(&app, Some(android.id())).await, 0);
}

#[tokio::test]
async fn the_queue_list_runs_in_sending_order_without_makeups_and_sandbox() {
    let app = TestApp::start().await;
    let (_, layout) = second_subject(&app).await;
    let first = submit(&app, &layout, &app.fixture.student, "pull/layout").await;
    add_makeup_and_sandbox(&app).await;
    let second = app.queued_submission(app.student(), "pull/backend").await;

    assert_eq!(list_ids(&app).await, [first, second]);
}

#[tokio::test]
async fn the_queue_list_holds_exactly_what_the_counter_counts() {
    let app = TestApp::start().await;
    let (_, layout) = second_subject(&app).await;
    submit(&app, &layout, &app.fixture.student, "pull/layout").await;
    app.queued_submission(app.student(), "pull/backend").await;
    add_makeup_and_sandbox(&app).await;

    assert_eq!(list_ids(&app).await.len() as u32, size(&app, None).await);
}

fn decide(
    app: &TestApp,
    submission: Uuid,
    decision: ReviewDecision,
    return_attempt: bool,
) -> ReviewSubmission {
    ReviewSubmission {
        submission_id: SubmissionId::from_uuid(submission),
        actor: app.teacher_actor(),
        decision,
        score: None,
        comment: "Оформите PR по шаблону".to_owned(),
        reviewed_at: Utc::now(),
        return_attempt,
    }
}

async fn extra_attempts(app: &TestApp, assignment: &Assignment, student: &User) -> u16 {
    db::submissions::load_histories(&app.pool, &[assignment.id().as_uuid()], student.id())
        .await
        .expect("read history")
        .remove(&assignment.id())
        .unwrap_or_default()
        .extra_attempts()
}

async fn status_of(app: &TestApp, submission: Uuid) -> SubmissionStatus {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .expect("read submission")
        .expect("submission exists")
        .status()
}

async fn single_attempt_assignment(app: &TestApp) -> Assignment {
    let assignment = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
        .title("Single")
        .max_attempts(1)
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;

    assignment
}

#[tokio::test]
async fn rework_rejects_and_returns_the_attempt() {
    let app = TestApp::start().await;
    let work = single_attempt_assignment(&app).await;
    let sent = submit(&app, &work, &app.fixture.student, "pull/1").await;

    let reviewed = app
        .application
        .reviews
        .review_submission(decide(&app, sent, ReviewDecision::Reject, true))
        .await
        .expect("the rework is saved");

    assert_eq!(reviewed.submission.status(), SubmissionStatus::Rejected);
    assert_eq!(extra_attempts(&app, &work, &app.fixture.student).await, 1);
    assert_eq!(
        reviewed.returned_attempt,
        Some(RemainingAttempts {
            remaining: 1,
            allowed: 2
        })
    );
}

#[tokio::test]
async fn a_returned_attempt_lets_the_student_send_again() {
    let app = TestApp::start().await;
    let work = single_attempt_assignment(&app).await;
    let sent = submit(&app, &work, &app.fixture.student, "pull/1").await;
    app.application
        .reviews
        .review_submission(decide(&app, sent, ReviewDecision::Reject, true))
        .await
        .expect("the rework is saved");

    app.submit(work.id().as_uuid(), app.student(), "pull/2")
        .await
        .created();
}

#[tokio::test]
async fn an_attempt_is_not_returned_with_an_acceptance() {
    let app = TestApp::start().await;
    let sent = app.queued_submission(app.student(), "pull/1").await;

    let error = app
        .application
        .reviews
        .review_submission(decide(&app, sent, ReviewDecision::Accept, true))
        .await
        .expect_err("an acceptance returns no attempt");

    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert_eq!(status_of(&app, sent).await, SubmissionStatus::PendingReview);
    assert_eq!(
        extra_attempts(&app, &app.fixture.assignment, &app.fixture.student).await,
        0
    );
}

#[tokio::test]
async fn an_attempt_is_not_returned_for_a_makeup() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    app.enable_makeup(3).await;
    let makeup = submit(&app, &closed, &app.fixture.student, "pull/makeup").await;

    let error = app
        .application
        .reviews
        .review_submission(decide(&app, makeup, ReviewDecision::Reject, true))
        .await
        .expect_err("a makeup returns no attempt");

    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert_eq!(
        status_of(&app, makeup).await,
        SubmissionStatus::PendingReview
    );
    assert_eq!(extra_attempts(&app, &closed, &app.fixture.student).await, 0);
}

#[tokio::test]
async fn a_rework_that_loses_the_race_returns_no_attempt() {
    let app = TestApp::start().await;
    let sent = app.queued_submission(app.student(), "pull/1").await;
    let mut reviewer = app.pool.begin().await.expect("begin a competing review");
    let blocker: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *reviewer)
        .await
        .expect("read backend pid");
    sqlx::query(
        "update submissions
         set status = 'accepted', reviewed_by = $2, reviewed_at = now(), review_comment = 'Принято'
         where id = $1",
    )
    .bind(sent)
    .bind(app.fixture.teacher.id().as_uuid())
    .execute(&mut *reviewer)
    .await
    .expect("review concurrently");

    let reviews = app.application.reviews.clone();
    let command = decide(&app, sent, ReviewDecision::Reject, true);
    let rework = tokio::spawn(async move { reviews.review_submission(command).await });
    wait_for_blocked_connection(&app.pool, blocker).await;
    reviewer
        .commit()
        .await
        .expect("commit the competing review");

    let error = rework
        .await
        .expect("the rework task finishes")
        .expect_err("the competing review wins");
    assert_eq!(error.code(), ErrorCode::SubmissionAlreadyReviewed);
    assert_eq!(
        extra_attempts(&app, &app.fixture.assignment, &app.fixture.student).await,
        0
    );
}

#[tokio::test]
async fn a_rework_enqueues_an_ordinary_rejection_notice() {
    let app = TestApp::start().await;
    let sent = app.queued_submission(app.student(), "pull/1").await;
    app.application
        .reviews
        .review_submission(decide(&app, sent, ReviewDecision::Reject, true))
        .await
        .expect("the rework is saved");

    let outcome: String = sqlx::query_scalar(
        "select outcome from review_notification_outbox where submission_id = $1",
    )
    .bind(sent)
    .fetch_one(&app.pool)
    .await
    .expect("the notice is enqueued");

    assert_eq!(outcome, "rejected");
}
