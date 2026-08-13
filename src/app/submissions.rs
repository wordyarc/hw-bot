use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{AssignmentId, DomainError, Submission, SubmissionId};

#[derive(Clone, Debug)]
pub struct SubmitHomework {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub content: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct QueuePlace {
    pub assignment_id: AssignmentId,
    pub subject_name: String,
    pub assignment_title: String,
    pub submitted_at: DateTime<Utc>,
    pub position: u32,
    pub total: u32,
}

impl From<db::submissions::PendingQueuePlace> for QueuePlace {
    fn from(place: db::submissions::PendingQueuePlace) -> Self {
        Self {
            assignment_id: place.assignment_id,
            subject_name: place.subject_name,
            assignment_title: place.assignment_title,
            submitted_at: place.submitted_at,
            position: place.position,
            total: place.total,
        }
    }
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

        let student = load_actor(&mut *tx, command.actor, "student").await?;

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

    pub async fn pending_queue(&self, actor: Actor) -> AppResult<Vec<QueuePlace>> {
        let student = load_actor(&self.pool, actor, "student").await?;

        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "read own review queue",
            }
            .into());
        }

        Ok(
            db::submissions::pending_queue_places(&self.pool, student.id())
                .await?
                .into_iter()
                .map(QueuePlace::from)
                .collect(),
        )
    }
}
