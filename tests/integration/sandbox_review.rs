use chrono::{DateTime, Duration, Utc};
use hw_bot::app::{Actor, ChangeTerms, ReviewSandbox};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, DeductedFrom, Grading, MakeupTerms, ReviewDecision, Submission, SubmissionId,
    SubmissionStatus, TeachingCourse, User,
};
use hw_bot::test_support::assignment;
use serde_json::json;
use uuid::Uuid;

use crate::harness::{TestApp, insert_assignment, submission_id, wait_for_blocked_connection};

const COMMENT: &str = "Учебная сдача";

struct TwoSandboxCourses {
    own: Assignment,
    foreign: Assignment,
    foreign_student: User,
}

async fn two_sandbox_courses(app: &TestApp) -> TwoSandboxCourses {
    let sandbox = app.add_sandbox_subject().await;
    let own = app.add_course_on(&sandbox).await;
    let foreign = app.add_foreign_course_on(&sandbox).await;
    let foreign_student = app.add_student_to_group("Foreign", &foreign.group).await;

    TwoSandboxCourses {
        own: app
            .add_published_assignment(&own, &app.fixture.teacher, "Trial")
            .await,
        foreign: app
            .add_published_assignment(&foreign.course, &foreign.teacher, "Foreign trial")
            .await,
        foreign_student,
    }
}

async fn add_trial(app: &TestApp) -> Assignment {
    let sandbox = app.add_sandbox_course().await;

    app.add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await
}

async fn submit(app: &TestApp, assignment: &Assignment, student: &User) -> Uuid {
    submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            student.telegram_user_id().as_i64(),
            "pull/trial",
        )
        .await
        .created(),
    )
}

fn decide_all(app: &TestApp, decision: ReviewDecision, cutoff: DateTime<Utc>) -> ReviewSandbox {
    ReviewSandbox {
        actor: app.teacher_actor(),
        decision,
        cutoff,
        comment: COMMENT.to_owned(),
        reviewed_at: Utc::now(),
    }
}

async fn review_sandbox(app: &TestApp, decision: ReviewDecision) -> usize {
    app.application
        .reviews
        .review_sandbox(decide_all(app, decision, Utc::now()))
        .await
        .expect("the bulk decision succeeds")
}

async fn find(app: &TestApp, submission: Uuid) -> Submission {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .expect("read submission")
        .expect("submission exists")
}

async fn sandbox_ids(app: &TestApp, actor: Actor) -> Vec<Uuid> {
    app.application
        .reviews
        .sandbox_page(actor, Default::default())
        .await
        .expect("the sandbox is readable")
        .items
        .iter()
        .map(|item| item.submission_id.as_uuid())
        .collect()
}

#[tokio::test]
async fn the_sandbox_list_holds_only_sandbox_submissions_of_own_courses() {
    let app = TestApp::start().await;
    let courses = two_sandbox_courses(&app).await;
    app.queued_submission(app.student(), "pull/regular").await;
    let own = submit(&app, &courses.own, &app.fixture.student).await;
    submit(&app, &courses.foreign, &courses.foreign_student).await;

    assert_eq!(sandbox_ids(&app, app.teacher_actor()).await, [own]);
}

#[tokio::test]
async fn an_admin_sees_every_sandbox_course_from_the_earliest() {
    let app = TestApp::start().await;
    let courses = two_sandbox_courses(&app).await;
    let first = submit(&app, &courses.foreign, &courses.foreign_student).await;
    let second = submit(&app, &courses.own, &app.fixture.student).await;
    let admin = app.add_admin().await;

    assert_eq!(
        sandbox_ids(&app, Actor::new(admin.id())).await,
        [first, second]
    );
}

#[tokio::test]
async fn the_sandbox_count_filters_like_the_list() {
    let app = TestApp::start().await;
    let courses = two_sandbox_courses(&app).await;
    app.queued_submission(app.student(), "pull/regular").await;
    submit(&app, &courses.own, &app.fixture.student).await;
    submit(&app, &courses.foreign, &courses.foreign_student).await;
    let admin = app.add_admin().await;

    for actor in [app.teacher_actor(), Actor::new(admin.id())] {
        let size = app
            .application
            .reviews
            .sandbox_size(actor, None)
            .await
            .expect("the sandbox is countable");

        assert_eq!(size as usize, sandbox_ids(&app, actor).await.len());
    }
}

#[tokio::test]
async fn rejecting_all_rejects_and_notifies_every_sandbox_submission() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let trainee = app.add_student("Trainee").await;
    let first = submit(&app, &trial, &app.fixture.student).await;
    let second = submit(&app, &trial, &trainee).await;

    let reviewed = review_sandbox(&app, ReviewDecision::Reject).await;

    assert_eq!(reviewed, 2);
    for submission in [first, second] {
        let saved = find(&app, submission).await;
        assert_eq!(saved.status(), SubmissionStatus::Rejected);
        assert_eq!(saved.review_comment().unwrap().as_str(), COMMENT);
    }
    let mut recipients: Vec<i64> = sqlx::query_scalar(
        "select recipient_telegram_user_id from review_notification_outbox
         where outcome = 'rejected' and review_comment = $1",
    )
    .bind(COMMENT)
    .fetch_all(&app.pool)
    .await
    .expect("read notifications");
    recipients.sort_unstable();
    assert_eq!(
        recipients,
        [app.student(), trainee.telegram_user_id().as_i64()]
    );
}

#[tokio::test]
async fn a_student_resubmits_after_a_bulk_rejection() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    submit(&app, &trial, &app.fixture.student).await;
    review_sandbox(&app, ReviewDecision::Reject).await;

    let body = app
        .submit(trial.id().as_uuid(), app.student(), "pull/again")
        .await
        .created();

    assert_eq!(body["attempt_number"], json!(2));
}

#[tokio::test]
async fn bulk_acceptance_gives_the_maximum_minus_the_late_penalty() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let late = assignment(&sandbox, &app.fixture.teacher)
        .title("Late trial")
        .created_at(Utc::now() - Duration::days(2))
        .deadline_at(Utc::now() - Duration::hours(12))
        .scored(10)
        .late_policy(1, 2, 4, DeductedFrom::WorkScore)
        .published()
        .build();
    insert_assignment(&app.pool, &late).await;
    let submission = submit(&app, &late, &app.fixture.student).await;

    review_sandbox(&app, ReviewDecision::Accept).await;

    let grade = find(&app, submission)
        .await
        .grade()
        .expect("a scored acceptance has a grade");
    assert_eq!(
        (grade.raw_score(), grade.penalty(), grade.work_score()),
        (10, 2, 8)
    );
    let notified = db::notifications::claim_next(
        &app.pool,
        Utc::now(),
        Uuid::new_v4(),
        Utc::now() + Duration::seconds(10),
    )
    .await
    .expect("claim notification")
    .expect("notification is enqueued");
    assert_eq!(notified.grade, Some(grade));
}

#[tokio::test]
async fn bulk_acceptance_of_a_pass_fail_trial_has_no_score() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit(&app, &trial, &app.fixture.student).await;

    review_sandbox(&app, ReviewDecision::Accept).await;

    let saved = find(&app, submission).await;
    assert_eq!(saved.status(), SubmissionStatus::Accepted);
    assert!(saved.grade().is_none());
}

#[tokio::test]
async fn a_submission_after_the_cutoff_stays_pending() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let trainee = app.add_student("Trainee").await;
    let before = submit(&app, &trial, &app.fixture.student).await;
    let cutoff = Utc::now();
    let after = submit(&app, &trial, &trainee).await;

    let reviewed = app
        .application
        .reviews
        .review_sandbox(decide_all(&app, ReviewDecision::Reject, cutoff))
        .await
        .expect("the bulk decision succeeds");

    assert_eq!(reviewed, 1);
    assert_eq!(
        find(&app, before).await.status(),
        SubmissionStatus::Rejected
    );
    assert!(find(&app, after).await.is_pending_review());
}

#[tokio::test]
async fn regular_submissions_are_left_pending() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let regular = app.queued_submission(app.student(), "pull/regular").await;
    submit(&app, &trial, &app.fixture.student).await;

    review_sandbox(&app, ReviewDecision::Accept).await;

    assert!(find(&app, regular).await.is_pending_review());
}

#[tokio::test]
async fn a_submission_reviewed_meanwhile_keeps_its_result() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let trainee = app.add_student("Trainee").await;
    let manual = submit(&app, &trial, &app.fixture.student).await;
    let bulk = submit(&app, &trial, &trainee).await;
    let mut reviewer = app.pool.begin().await.expect("begin a manual review");
    let blocker: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *reviewer)
        .await
        .expect("read backend pid");
    sqlx::query(
        "update submissions
         set status = 'accepted', reviewed_by = $2, reviewed_at = now(), review_comment = 'Вручную'
         where id = $1",
    )
    .bind(manual)
    .bind(app.fixture.teacher.id().as_uuid())
    .execute(&mut *reviewer)
    .await
    .expect("review manually");

    let reviews = app.application.reviews.clone();
    let command = decide_all(&app, ReviewDecision::Reject, Utc::now());
    let decision = tokio::spawn(async move { reviews.review_sandbox(command).await });
    wait_for_blocked_connection(&app.pool, blocker).await;
    reviewer.commit().await.expect("commit the manual review");

    let reviewed = decision
        .await
        .expect("the bulk decision task completes")
        .expect("the bulk decision succeeds");
    assert_eq!(reviewed, 1);
    let kept = find(&app, manual).await;
    assert_eq!(kept.status(), SubmissionStatus::Accepted);
    assert_eq!(kept.review_comment().unwrap().as_str(), "Вручную");
    assert_eq!(find(&app, bulk).await.status(), SubmissionStatus::Rejected);
}

#[tokio::test]
async fn nothing_to_decide_changes_nothing() {
    let app = TestApp::start().await;
    add_trial(&app).await;

    let reviewed = review_sandbox(&app, ReviewDecision::Reject).await;

    assert_eq!(reviewed, 0);
    let notifications: i64 = sqlx::query_scalar("select count(*) from review_notification_outbox")
        .fetch_one(&app.pool)
        .await
        .expect("count notifications");
    assert_eq!(notifications, 0);
}

async fn sandbox_makeup(app: &TestApp) -> (TeachingCourse, Assignment, Uuid) {
    let sandbox = app.add_sandbox_course().await;
    let ten = Grading::scored(10).unwrap();
    let trial = app
        .add_makeup_assignment_on(
            &sandbox,
            "Closed trial",
            ten,
            MakeupTerms::scored(6, ten).unwrap(),
            Utc::now() - Duration::days(1),
        )
        .await;
    app.enable_makeup_on(&sandbox, 3).await;
    let makeup = submit(app, &trial, &app.fixture.student).await;

    (sandbox, trial, makeup)
}

#[tokio::test]
async fn bulk_acceptance_credits_a_makeup_by_its_terms_instead_of_the_maximum() {
    let app = TestApp::start().await;
    let (_, _, makeup) = sandbox_makeup(&app).await;

    assert_eq!(review_sandbox(&app, ReviewDecision::Accept).await, 1);

    let grade = find(&app, makeup)
        .await
        .grade()
        .expect("the makeup is credited");
    assert_eq!((grade.work_score(), grade.max_score()), (6, 10));
}

#[tokio::test]
async fn bulk_acceptance_leaves_a_makeup_with_withdrawn_terms_pending_and_uncounted() {
    let app = TestApp::start().await;
    let (sandbox, trial, makeup) = sandbox_makeup(&app).await;
    let regular_trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let regular = submit(&app, &regular_trial, &app.fixture.student).await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: trial.id(),
            actor: app.teacher_actor(),
            deadline_at: trial.deadline_at(),
            late_policy: None,
            makeup: Some(MakeupTerms::NotOffered),
            expected_revision: 0,
        })
        .await
        .unwrap();

    assert_eq!(review_sandbox(&app, ReviewDecision::Accept).await, 1);

    assert_eq!(
        find(&app, makeup).await.status(),
        SubmissionStatus::PendingReview
    );
    assert_eq!(
        find(&app, regular).await.status(),
        SubmissionStatus::Accepted
    );
}
