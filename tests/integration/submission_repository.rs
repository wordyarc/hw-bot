use chrono::{DateTime, Duration, Utc};
use hw_bot::app::{ErrorCode, SubmitHomework};
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

    let history = submissions::load_histories(
        &app.pool,
        &[app.fixture.assignment.id().as_uuid()],
        app.fixture.student.id(),
    )
    .await?
    .remove(&app.fixture.assignment.id())
    .expect("requested assignment has a history even without submissions");

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

    let stored = submissions::load_histories(
        &app.pool,
        &[app.fixture.assignment.id().as_uuid()],
        app.fixture.student.id(),
    )
    .await?
    .remove(&app.fixture.assignment.id())
    .expect("requested assignment history");

    assert_eq!(stored, history().pending().submissions(1).build());

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
        None,
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

    let stored = submissions::load_histories(
        &app.pool,
        &[app.fixture.assignment.id().as_uuid()],
        app.fixture.student.id(),
    )
    .await?
    .remove(&app.fixture.assignment.id())
    .expect("requested assignment history");
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

    first_tx.commit().await?;

    let mut repeated_tx = app.pool.begin().await?;
    let repeated =
        submissions::lock_next_pending_for_reviewer(&mut repeated_tx, &app.fixture.teacher)
            .await?
            .expect("the committed queue read must not reserve its submission");
    assert_eq!(
        repeated.id(),
        head.id(),
        "the head becomes available again as soon as the read transaction ends"
    );

    repeated_tx.rollback().await?;
    second_tx.rollback().await?;

    Ok(())
}

/// Исход гонки определяет только `update ... where status = 'pending_review'`.
/// Второму писателю возвращается `SubmissionIsNotPendingReview`.
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
        None,
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

#[tokio::test]
async fn a_double_tap_reports_the_same_error_as_the_up_front_check() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let first = submission(&app.fixture.assignment, &app.fixture.student).build();
    submissions::insert(&app.pool, &first).await?;

    for used in [0, 1] {
        let double_tap = submission(&app.fixture.assignment, &app.fixture.student)
            .history(history().used(used).build())
            .build();
        let lost = submissions::insert(&app.pool, &double_tap).await;
        assert!(
            matches!(
                lost,
                Err(DbError::RuleViolation(
                    DomainError::SubmissionAlreadyPendingReview
                ))
            ),
            "both a duplicate attempt number and another pending attempt must report the same rule; used={used}, result={lost:?}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn storage_rejects_a_second_accepted_attempt_of_the_same_assignment() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    for used in [0, 1] {
        let mut accepted = submission(&app.fixture.assignment, &app.fixture.student)
            .history(history().used(used).build())
            .build();
        accepted.accept(
            &app.fixture.assignment,
            &app.fixture.course,
            &app.fixture.teacher,
            None,
            "Accepted",
            reviewed_at(&app.fixture.assignment),
        )?;
        let result = submissions::insert(&app.pool, &accepted).await;
        if used == 0 {
            result?;
        } else {
            assert!(
                matches!(
                    result,
                    Err(DbError::RuleViolation(
                        DomainError::SubmissionAlreadyAccepted
                    ))
                ),
                "{result:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_double_submit_creates_one_pending_submission() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let command = SubmitHomework {
        assignment_id: app.fixture.assignment.id(),
        actor: app.student_actor(),
        content: "pull/1".to_owned(),
        submitted_at: Utc::now(),
        allow_makeup: false,
    };

    let (first, second) = tokio::join!(
        app.application.submissions.submit_homework(command.clone()),
        app.application.submissions.submit_homework(command),
    );

    assert_eq!(
        first.is_ok() as u8 + second.is_ok() as u8,
        1,
        "exactly one concurrent submit must be created; first={first:?}, second={second:?}"
    );
    let error = first.err().or_else(|| second.err()).unwrap();
    assert_eq!(error.code(), ErrorCode::SubmissionPendingReview);

    let count: i64 = sqlx::query_scalar(
        "select count(*) from submissions where assignment_id = $1 and student_id = $2",
    )
    .bind(app.fixture.assignment.id().as_uuid())
    .bind(app.fixture.student.id().as_uuid())
    .fetch_one(&app.pool)
    .await?;
    assert_eq!(count, 1);

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
