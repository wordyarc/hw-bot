mod common;

use chrono::{DateTime, Duration, Utc};
use common::{Fixture, postgres_pool};
use hw_bot::db::submissions::SubmissionRepository;
use hw_bot::domain::{
    Assignment, Submission, SubmissionHistory, SubmissionId, SubmissionStatus, User,
};
use sqlx::PgPool;

#[tokio::test]
async fn submission_repository_persists_history_reviews_and_queue_order() -> anyhow::Result<()> {
    let (_postgres, pool) = postgres_pool().await?;
    hw_bot::db::run_migrations(&pool).await?;

    let fixture = Fixture::insert(&pool).await?;
    let repo = SubmissionRepository::new();

    let history = repo
        .load_history(&pool, fixture.assignment.id(), fixture.student.id())
        .await?;
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

    repo.insert(&pool, &submission).await?;

    let loaded = repo.find_by_id(&pool, submission.id()).await?.unwrap();
    assert_eq!(loaded.id(), submission.id());
    assert_eq!(loaded.status(), SubmissionStatus::PendingReview);
    assert_eq!(loaded.content().as_str(), submission.content().as_str());

    let history = repo
        .load_history(&pool, fixture.assignment.id(), fixture.student.id())
        .await?;
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
    repo.save_review_result(&pool, &reviewed).await?;

    let loaded = repo.find_by_id(&pool, reviewed.id()).await?.unwrap();
    assert_eq!(loaded.status(), SubmissionStatus::Accepted);
    assert_eq!(loaded.reviewed_by(), Some(fixture.teacher.id()));
    assert_eq!(loaded.review_comment().unwrap().as_str(), "95/100");

    let history = repo
        .load_history(&pool, fixture.assignment.id(), fixture.student.id())
        .await?;
    assert_eq!(history, SubmissionHistory::new(1, 0, true, false));

    let second_student = fixture
        .insert_student(&pool, 1002, "Second student")
        .await?;
    let third_student = fixture.insert_student(&pool, 1003, "Third student").await?;
    let later_submission = submit_for_student(
        &repo,
        &pool,
        &fixture.assignment,
        &second_student,
        submitted_at + Duration::seconds(2),
    )
    .await?;
    let earlier_submission = submit_for_student(
        &repo,
        &pool,
        &fixture.assignment,
        &third_student,
        submitted_at + Duration::seconds(1),
    )
    .await?;

    let mut tx = pool.begin().await?;
    let next = repo
        .lock_next_pending_for_reviewer(&mut tx, &fixture.teacher)
        .await?
        .unwrap();
    tx.rollback().await?;

    assert_eq!(next.id(), earlier_submission.id());
    assert_ne!(next.id(), later_submission.id());

    Ok(())
}

async fn submit_for_student(
    repo: &SubmissionRepository,
    pool: &PgPool,
    assignment: &Assignment,
    student: &User,
    submitted_at: DateTime<Utc>,
) -> anyhow::Result<Submission> {
    let history = repo
        .load_history(pool, assignment.id(), student.id())
        .await?;
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

    repo.insert(pool, &submission).await?;

    Ok(submission)
}
