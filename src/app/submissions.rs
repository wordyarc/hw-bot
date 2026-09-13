use crate::pagination::{Page, PageRequest};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::access::load_student;
use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{AssignmentId, Submission, SubmissionId};
use crate::read_models::QueuePlace;

#[derive(Clone, Debug)]
pub struct SubmitHomework {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
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

    pub async fn pending_queue(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<QueuePlace>> {
        let student = load_student(&self.pool, actor, "read own review queue").await?;

        Ok(Page::from_rows(
            db::submissions::pending_queue_places(&self.pool, student.id(), page, None).await?,
            page,
        ))
    }

    pub async fn queue_place(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<Option<QueuePlace>> {
        let student = load_student(&self.pool, actor, "read own review queue").await?;
        Ok(db::submissions::pending_queue_places(
            &self.pool,
            student.id(),
            PageRequest::default(),
            Some(assignment_id),
        )
        .await?
        .into_iter()
        .next())
    }
}
