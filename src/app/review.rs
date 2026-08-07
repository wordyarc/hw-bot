use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;

use crate::app::{AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, DomainError, ReviewDecision, Submission, SubmissionId, TelegramUserId, User,
};

async fn load_reviewer<'e, E>(executor: E, telegram_user_id: TelegramUserId) -> AppResult<User>
where
    E: PgExecutor<'e>,
{
    let reviewer = db::users::find_by_telegram_user_id(executor, telegram_user_id)
        .await?
        .required("reviewer")?;

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
    pub reviewer_telegram_user_id: TelegramUserId,
    pub decision: ReviewDecision,
    pub comment: String,
    pub reviewed_at: DateTime<Utc>,
}

/// Сдача вместе с контекстом: по одной ссылке на pull request решение принять нельзя, нужны текст
/// задания и имя студента.
#[derive(Clone, Debug)]
pub struct ReviewQueueItem {
    pub submission: Submission,
    pub assignment: Assignment,
    pub student: User,
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
    pub async fn take_next_from_queue(
        &self,
        reviewer_telegram_user_id: TelegramUserId,
    ) -> AppResult<Option<ReviewQueueItem>> {
        let mut tx = self.pool.begin().await?;

        let reviewer = load_reviewer(&mut *tx, reviewer_telegram_user_id).await?;

        let Some(submission) =
            db::submissions::lock_next_pending_for_reviewer(&mut tx, &reviewer).await?
        else {
            return Ok(None);
        };

        let assignment = db::assignments::find_by_id(&mut *tx, submission.assignment_id())
            .await?
            .required("assignment")?;

        let student = db::users::find_by_id(&mut *tx, submission.student_id())
            .await?
            .required("student")?;

        tx.commit().await?;

        Ok(Some(ReviewQueueItem {
            submission,
            assignment,
            student,
        }))
    }

    /// Сколько сдач ждёт этого ревьюера. Бот не решает, звать ли
    /// `take_next`: очередь может опустеть между двумя запросами
    pub async fn queue_size(&self, reviewer_telegram_user_id: TelegramUserId) -> AppResult<u32> {
        let reviewer = load_reviewer(&self.pool, reviewer_telegram_user_id).await?;

        Ok(db::submissions::count_pending_for_reviewer(&self.pool, &reviewer).await?)
    }

    pub async fn review_submission(&self, command: ReviewSubmission) -> AppResult<Submission> {
        let mut tx = self.pool.begin().await?;

        let reviewer =
            db::users::find_by_telegram_user_id(&mut *tx, command.reviewer_telegram_user_id)
                .await?
                .required("reviewer")?;

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
                command.comment,
                command.reviewed_at,
            )?,
            ReviewDecision::Reject => submission.reject(
                &assignment,
                &teaching_course,
                &reviewer,
                command.comment,
                command.reviewed_at,
            )?,
        }

        db::submissions::save_review_result(&mut *tx, &submission).await?;

        tx.commit().await?;

        Ok(submission)
    }
}
