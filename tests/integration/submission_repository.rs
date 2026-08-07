use chrono::{DateTime, Duration, Utc};
use hw_bot::db::submissions;
use hw_bot::db::{DbError, DbResult};
use hw_bot::domain::{
    Assignment, DomainError, Submission, SubmissionHistory, SubmissionStatus, User,
};
use hw_bot::test_support::{history, submission};
use sqlx::PgPool;

use crate::harness::TestApp;

#[tokio::test]
async fn history_is_empty_for_a_student_without_submissions() -> anyhow::Result<()> {
    let app = TestApp::start().await;

    let history = submissions::load_history(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await?;

    assert_eq!(history, SubmissionHistory::default());

    Ok(())
}

#[tokio::test]
async fn insert_and_find_round_trip_preserves_the_submission() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let submission = submission(&app.fixture.assignment, &app.fixture.student).build();

    submissions::insert(&app.pool, &submission).await?;

    let loaded = submissions::find_by_id(&app.pool, submission.id())
        .await?
        .expect("the submission is stored");

    assert_eq!(loaded, submission);
    assert_eq!(loaded.status(), SubmissionStatus::PendingReview);

    Ok(())
}

#[tokio::test]
async fn pending_submission_blocks_but_does_not_spend_an_attempt() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let submission = submission(&app.fixture.assignment, &app.fixture.student).build();
    submissions::insert(&app.pool, &submission).await?;

    let stored = submissions::load_history(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await?;

    assert_eq!(stored, history().pending().build());

    Ok(())
}

#[tokio::test]
async fn accepted_review_is_persisted_and_spends_an_attempt() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let mut submission = submission(&app.fixture.assignment, &app.fixture.student).build();
    submissions::insert(&app.pool, &submission).await?;

    submission.accept(
        &app.fixture.assignment,
        &app.fixture.course,
        &app.fixture.teacher,
        "95/100",
        reviewed_at(&app.fixture.assignment),
    )?;
    submissions::save_review_result(&app.pool, &submission).await?;

    let loaded = submissions::find_by_id(&app.pool, submission.id())
        .await?
        .expect("the submission is stored");
    assert_eq!(loaded.status(), SubmissionStatus::Accepted);
    assert_eq!(loaded.reviewed_by(), Some(app.fixture.teacher.id()));
    assert_eq!(loaded.review_comment().unwrap().as_str(), "95/100");

    let stored = submissions::load_history(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await?;
    assert_eq!(stored, history().used(1).accepted().build());

    Ok(())
}

#[tokio::test]
async fn queue_is_ordered_by_submitted_at_not_by_insertion_order() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let second = app.add_student("Second student").await;
    let third = app.add_student("Third student").await;

    let later = insert_submission(&app.pool, &app.fixture.assignment, &second, 2).await?;
    let earlier = insert_submission(&app.pool, &app.fixture.assignment, &third, 1).await?;

    let mut tx = app.pool.begin().await?;
    let next = submissions::lock_next_pending_for_reviewer(&mut tx, &app.fixture.teacher)
        .await?
        .expect("the queue is not empty");
    tx.rollback().await?;

    assert_eq!(next.id(), earlier.id());
    assert_ne!(next.id(), later.id());

    Ok(())
}

#[tokio::test]
async fn concurrent_reviewers_take_different_submissions() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let other_student = app.add_student("Second student").await;
    insert_submission(&app.pool, &app.fixture.assignment, &app.fixture.student, 1).await?;
    insert_submission(&app.pool, &app.fixture.assignment, &other_student, 2).await?;

    let mut first_tx = app.pool.begin().await?;
    let head = submissions::lock_next_pending_for_reviewer(&mut first_tx, &app.fixture.teacher)
        .await?
        .expect("the queue is not empty");

    let mut second_tx = app.pool.begin().await?;
    let next = submissions::lock_next_pending_for_reviewer(&mut second_tx, &app.fixture.teacher)
        .await?
        .expect("the locked head must be skipped, not waited for");

    assert_ne!(
        head.id(),
        next.id(),
        "skip locked must hand out a different submission"
    );

    first_tx.rollback().await?;
    second_tx.rollback().await?;

    Ok(())
}

/// Единственный арбитр гонки - условный `update ... where status = 'pending_review'`.
/// Проигравший обязан получить `SubmissionIsNotPendingReview`, а не молча перезаписать решение.
#[tokio::test]
async fn the_loser_of_a_review_race_is_rejected_by_the_conditional_update() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let submission = submission(&app.fixture.assignment, &app.fixture.student).build();
    submissions::insert(&app.pool, &submission).await?;

    let mut winner_tx = app.pool.begin().await?;
    let mut loser_tx = app.pool.begin().await?;

    let mut winner = submissions::find_by_id(&mut *winner_tx, submission.id())
        .await?
        .expect("the submission is stored");
    let mut loser = submissions::find_by_id(&mut *loser_tx, submission.id())
        .await?
        .expect("the submission is stored");

    winner.accept(
        &app.fixture.assignment,
        &app.fixture.course,
        &app.fixture.teacher,
        "95/100",
        reviewed_at(&app.fixture.assignment),
    )?;
    loser.reject(
        &app.fixture.assignment,
        &app.fixture.course,
        &app.fixture.teacher,
        "Add tests.",
        reviewed_at(&app.fixture.assignment),
    )?;

    submissions::save_review_result(&mut *winner_tx, &winner).await?;
    winner_tx.commit().await?;

    let lost = submissions::save_review_result(&mut *loser_tx, &loser).await;
    loser_tx.rollback().await?;

    assert!(
        matches!(
            lost,
            Err(DbError::RuleViolation(
                DomainError::SubmissionIsNotPendingReview
            ))
        ),
        "expected a lost race, got {lost:?}"
    );

    let loaded = submissions::find_by_id(&app.pool, submission.id())
        .await?
        .expect("the submission is stored");
    assert_eq!(loaded.status(), SubmissionStatus::Accepted);

    Ok(())
}

/// Сдача в статусе `PendingReview` не может быть сохранена как результат ревью: без этой
/// проверки условный update снял бы статус и оставил сдачу без решения.
#[tokio::test]
async fn a_pending_submission_cannot_be_saved_as_a_review_result() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let submission = submission(&app.fixture.assignment, &app.fixture.student).build();
    submissions::insert(&app.pool, &submission).await?;

    let result = submissions::save_review_result(&app.pool, &submission).await;

    assert!(
        matches!(result, Err(DbError::Conflict { .. })),
        "expected a conflict, got {result:?}"
    );

    Ok(())
}

async fn insert_submission(
    pool: &PgPool,
    assignment: &Assignment,
    student: &User,
    hours_after_creation: i64,
) -> DbResult<Submission> {
    let submitted_at: DateTime<Utc> =
        assignment.created_at() + Duration::hours(hours_after_creation);
    let submission = submission(assignment, student)
        .submitted_at(submitted_at)
        .build();

    submissions::insert(pool, &submission).await?;

    Ok(submission)
}

fn reviewed_at(assignment: &Assignment) -> DateTime<Utc> {
    assignment.created_at() + Duration::hours(2)
}
