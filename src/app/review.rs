use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, DomainError, ReviewDecision, Subject, Submission, SubmissionId, User,
};
use crate::pagination::PageRequest;

async fn load_reviewer<'e, E>(executor: E, actor: Actor) -> AppResult<User>
where
    E: PgExecutor<'e>,
{
    let reviewer = load_actor(executor, actor, "reviewer").await?;

    if !reviewer.role().can_review_homework() {
        return Err(DomainError::PermissionDenied {
            action: "review homework",
        }
        .into());
    }

    Ok(reviewer)
}

#[derive(Clone, Debug)]
pub struct ReviewSubmission {
    pub submission_id: SubmissionId,
    pub actor: Actor,
    pub decision: ReviewDecision,
    pub score: Option<u16>,
    pub comment: String,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ReviewQueueItem {
    pub submission: Submission,
    pub assignment: Assignment,
    pub subject: Subject,
    pub student: User,
    pub previous_attempts: Vec<Submission>,
}

#[derive(Clone, Debug)]
pub struct ReviewService {
    pool: PgPool,
}

impl ReviewService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Выдача из очереди не эксклюзивна: блокировка живет до конца транзакции, а HTTP-запрос
    /// заканчивается раньше решения. Единственный арбитр - условный update в `save_review_result`,
    /// поэтому проигравший гонку ревьюер получит `SubmissionIsNotPendingReview`.
    pub async fn take_next_from_queue(&self, actor: Actor) -> AppResult<Option<ReviewQueueItem>> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, actor).await?;

        let Some(submission) =
            db::submissions::lock_next_pending_for_reviewer(&mut tx, &reviewer).await?
        else {
            return Ok(None);
        };

        let item = load_queue_item(&mut tx, submission).await?;
        tx.commit().await?;
        Ok(Some(item))
    }

    /// Сколько сдач ждёт этого ревьюера. Бот не решает, звать ли
    /// `take_next`: очередь может опустеть между двумя запросами
    pub async fn queue_size(&self, actor: Actor) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, actor).await?;

        Ok(db::submissions::count_pending_for_reviewer(&self.pool, &reviewer).await?)
    }

    pub async fn review_submission(&self, command: ReviewSubmission) -> AppResult<Submission> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, command.actor).await?;

        let mut submission = db::submissions::find_by_id(&mut *tx, command.submission_id)
            .await?
            .required("submission")?;

        let assignment = db::assignments::find_by_id(&mut *tx, submission.assignment_id())
            .await?
            .required("assignment")?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, assignment.teaching_course_id())
                .await?
                .required("teaching course")?;

        match command.decision {
            ReviewDecision::Accept => submission.accept(
                &assignment,
                &teaching_course,
                &reviewer,
                command.score,
                command.comment,
                command.reviewed_at,
            )?,
            ReviewDecision::Reject if command.score.is_some() => {
                return Err(DomainError::ScoreNotAllowed.into());
            }
            ReviewDecision::Reject => submission.reject(
                &assignment,
                &teaching_course,
                &reviewer,
                command.comment,
                command.reviewed_at,
            )?,
        }

        db::submissions::save_review_result(&mut *tx, &submission).await?;

        let student = db::users::find_by_id(&mut *tx, submission.student_id())
            .await?
            .required("student")?;
        let subject = db::academic::find_subject_by_id(&mut *tx, teaching_course.subject_id())
            .await?
            .required("subject")?;
        db::notifications::enqueue_review_result(
            &mut *tx,
            &submission,
            &assignment,
            &subject,
            &student,
        )
        .await?;

        tx.commit().await?;

        Ok(submission)
    }

    pub async fn find_for_review(
        &self,
        actor: Actor,
        submission_id: SubmissionId,
    ) -> AppResult<ReviewQueueItem> {
        let mut connection = self.pool.acquire().await?;
        let reviewer = load_reviewer(&mut *connection, actor).await?;
        let submission = db::submissions::find_by_id(&mut *connection, submission_id)
            .await?
            .required("submission")?;
        let item = load_queue_item(&mut connection, submission).await?;
        let course = db::academic::find_teaching_course_by_id(
            &mut *connection,
            item.assignment.teaching_course_id(),
        )
        .await?
        .required("teaching course")?;
        item.assignment
            .ensure_managed_by(&course, &reviewer, "review homework")?;
        if !item.submission.is_pending_review() {
            return Err(DomainError::SubmissionIsNotPendingReview.into());
        }
        Ok(item)
    }
}

async fn load_queue_item(
    connection: &mut PgConnection,
    submission: Submission,
) -> AppResult<ReviewQueueItem> {
    let assignment = db::assignments::find_by_id(&mut *connection, submission.assignment_id())
        .await?
        .required("assignment")?;
    let subject = db::academic::find_subject_by_id(&mut *connection, assignment.subject_id())
        .await?
        .required("subject")?;
    let student = db::users::find_by_id(&mut *connection, submission.student_id())
        .await?
        .required("student")?;
    let mut previous_attempts: Vec<_> = db::submissions::list_for_student(
        &mut *connection,
        assignment.id(),
        student.id(),
        PageRequest::default(),
    )
    .await?
    .into_iter()
    .filter(|attempt| attempt.attempt_number() < submission.attempt_number())
    .collect();
    previous_attempts.reverse();
    Ok(ReviewQueueItem {
        submission,
        assignment,
        subject,
        student,
        previous_attempts,
    })
}
