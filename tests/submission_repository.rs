mod common;

use chrono::{DateTime, Duration, Utc};
use common::TestApp;
use hw_bot::db::submissions;
use hw_bot::domain::{
    Assignment, Submission, SubmissionHistory, SubmissionId, SubmissionStatus, User,
};
use sqlx::PgPool;

#[tokio::test]
async fn submission_repository_persists_history_reviews_and_queue_order() -> anyhow::Result<()> {
    let test_app = TestApp::start().await?;
    let (pool, fixture) = (&test_app.pool, &test_app.fixture);

    let history =
        submissions::load_history(pool, fixture.assignment.id(), fixture.student.id()).await?;
    assert_eq!(history, SubmissionHistory::default());

    let submitted_at = Utc::now();
    let submission = Submission::submit(
        SubmissionId::new(),
        &fixture.assignment,
        &fixture.student,
        history,
        "https://github.com/course/hw/pull/1",
        submitted_at,
    )?;

    submissions::insert(pool, &submission).await?;

    let loaded = submissions::find_by_id(pool, submission.id())
        .await?
        .unwrap();
    assert_eq!(loaded.id(), submission.id());
    assert_eq!(loaded.status(), SubmissionStatus::PendingReview);
    assert_eq!(loaded.content().as_str(), submission.content().as_str());

    let history =
        submissions::load_history(pool, fixture.assignment.id(), fixture.student.id()).await?;
    assert_eq!(
        history,
        SubmissionHistory::new(0, 0, false, true),
        "pending submission blocks next attempt but does not spend it yet"
    );

    let mut reviewed = loaded;
    reviewed.accept(
        &fixture.assignment,
        &fixture.course,
        &fixture.teacher,
        "95/100",
        Utc::now(),
    )?;
    submissions::save_review_result(pool, &reviewed).await?;

    let loaded = submissions::find_by_id(pool, reviewed.id()).await?.unwrap();
    assert_eq!(loaded.status(), SubmissionStatus::Accepted);
    assert_eq!(loaded.reviewed_by(), Some(fixture.teacher.id()));
    assert_eq!(loaded.review_comment().unwrap().as_str(), "95/100");

    let history =
        submissions::load_history(pool, fixture.assignment.id(), fixture.student.id()).await?;
    assert_eq!(history, SubmissionHistory::new(1, 0, true, false));

    let second_student = fixture.insert_student(pool, 1002, "Second student").await?;
    let third_student = fixture.insert_student(pool, 1003, "Third student").await?;
    let later_submission = submit_for_student(
        pool,
        &fixture.assignment,
        &second_student,
        submitted_at + Duration::seconds(2),
    )
    .await?;
    let earlier_submission = submit_for_student(
        pool,
        &fixture.assignment,
        &third_student,
        submitted_at + Duration::seconds(1),
    )
    .await?;

    let mut tx = pool.begin().await?;
    let next = submissions::lock_next_pending_for_reviewer(&mut tx, &fixture.teacher)
        .await?
        .unwrap();
    tx.rollback().await?;

    assert_eq!(next.id(), earlier_submission.id());
    assert_ne!(next.id(), later_submission.id());

    Ok(())
}

async fn submit_for_student(
    pool: &PgPool,
    assignment: &Assignment,
    student: &User,
    submitted_at: DateTime<Utc>,
) -> anyhow::Result<Submission> {
    let history = submissions::load_history(pool, assignment.id(), student.id()).await?;
    let submission = Submission::submit(
        SubmissionId::new(),
        assignment,
        student,
        history,
        format!(
            "https://github.com/course/hw/pull/{}",
            student.telegram_user_id().as_i64()
        ),
        submitted_at,
    )?;

    submissions::insert(pool, &submission).await?;

    Ok(submission)
}
