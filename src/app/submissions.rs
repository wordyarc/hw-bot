use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::{AppResult, Required};
use crate::db;
use crate::domain::{AssignmentId, Submission, SubmissionId, TelegramUserId};

#[derive(Clone, Debug)]
pub struct SubmitHomework {
    pub assignment_id: AssignmentId,
    pub telegram_user_id: TelegramUserId,
    pub content: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct SubmissionService {
    pool: PgPool,
}

impl SubmissionService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn submit_homework(&self, command: SubmitHomework) -> AppResult<Submission> {
        let mut tx = self.pool.begin().await?;

        let student = db::users::find_by_telegram_user_id(&mut *tx, command.telegram_user_id)
            .await?
            .required("student")?;

        let assignment = db::assignments::find_by_id(&mut *tx, command.assignment_id)
            .await?
            .required("assignment")?;

        let history =
            db::submissions::load_history(&mut *tx, assignment.id(), student.id()).await?;

        let submission = Submission::submit(
            SubmissionId::new(),
            &assignment,
            &student,
            history,
            command.content,
            command.submitted_at,
        )?;

        db::submissions::insert(&mut *tx, &submission).await?;

        tx.commit().await?;

        Ok(submission)
    }
}
