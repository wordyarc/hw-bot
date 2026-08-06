use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::{AppError, AppResult};
use crate::db::assignments::AssignmentRepository;
use crate::db::submissions::SubmissionRepository;
use crate::db::users::UserRepository;
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
    users: UserRepository,
    assignments: AssignmentRepository,
    submissions: SubmissionRepository,
}

impl SubmissionService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            users: UserRepository::new(),
            assignments: AssignmentRepository::new(),
            submissions: SubmissionRepository::new(),
        }
    }

    pub async fn submit_homework(&self, command: SubmitHomework) -> AppResult<Submission> {
        let mut tx = self.pool.begin().await.map_err(crate::db::DbError::from)?;

        let student = self
            .users
            .find_by_telegram_user_id(&mut *tx, command.telegram_user_id)
            .await?
            .ok_or(AppError::NotFound {
                resource: "student",
            })?;

        let assignment = self
            .assignments
            .find_by_id(&mut *tx, command.assignment_id)
            .await?
            .ok_or(AppError::NotFound {
                resource: "assignment",
            })?;

        let history = self
            .submissions
            .load_history(&mut *tx, assignment.id(), student.id())
            .await?;

        let submission = Submission::submit(
            SubmissionId::new(),
            &assignment,
            &student,
            history,
            command.content,
            command.submitted_at,
        )?;

        self.submissions.insert(&mut *tx, &submission).await?;

        tx.commit().await.map_err(crate::db::DbError::from)?;

        Ok(submission)
    }
}
