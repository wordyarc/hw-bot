use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::access::load_student;
use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{
    Acceptance, AssignmentId, MakeupBalance, MakeupRequest, Submission, SubmissionId,
    SubmissionKind,
};
use crate::pagination::{Page, PageRequest};
use crate::read_models::QueuePlace;

#[derive(Clone, Debug)]
pub struct SubmitHomework {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub content: String,
    pub submitted_at: DateTime<Utc>,
    pub allow_makeup: bool,
}

#[derive(Clone, Debug)]
pub struct SubmittedHomework {
    pub submission: Submission,
    pub makeup_balance: Option<MakeupBalance>,
}

#[derive(Clone, Debug)]
pub struct SubmissionService {
    pool: PgPool,
}

impl SubmissionService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn submit_homework(&self, command: SubmitHomework) -> AppResult<SubmittedHomework> {
        let mut tx = self.pool.begin().await?;

        let student = load_actor(&mut *tx, command.actor, "student").await?;

        let assignment = db::assignments::find_by_id(&mut *tx, command.assignment_id)
            .await?
            .required("assignment")?;

        let history =
            db::submissions::load_histories(&mut *tx, &[assignment.id().as_uuid()], student.id())
                .await?
                .remove(&assignment.id())
                .unwrap_or_default();

        let course_id = assignment.teaching_course_id();
        let mut makeup = MakeupRequest {
            course: db::makeup::find_course(&mut *tx, course_id).await?,
            consent: command.allow_makeup,
            balance: MakeupBalance::default(),
        };
        let acceptance =
            Submission::admit(&assignment, &student, history, command.submitted_at, makeup)?;
        if acceptance != Acceptance::Regular {
            db::makeup::lock_pool(&mut tx, course_id, student.id()).await?;
            let pool = db::makeup::pool_for_student(&mut *tx, course_id, student.id()).await?;
            makeup.course = pool.map(|pool| pool.course);
            makeup.balance = pool.map(|pool| pool.balance).unwrap_or_default();
        }

        let submission = Submission::submit(
            SubmissionId::new(),
            &assignment,
            &student,
            history,
            command.content,
            command.submitted_at,
            makeup,
        )?;

        db::submissions::insert(&mut *tx, &submission).await?;

        tx.commit().await?;

        Ok(SubmittedHomework {
            makeup_balance: (submission.kind() == SubmissionKind::Makeup)
                .then(|| makeup.balance.spend()),
            submission,
        })
    }

    pub async fn pending_queue(
        &self,
        actor: Actor,
        page: PageRequest,
    ) -> AppResult<Page<QueuePlace>> {
        let student = load_student(&self.pool, actor, "read own review queue").await?;

        Ok(Page::from_rows(
            db::submissions::pending_queue_page(&self.pool, student.id(), page).await?,
            page,
        ))
    }

    pub async fn queue_place(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<Option<QueuePlace>> {
        let student = load_student(&self.pool, actor, "read own review queue").await?;
        Ok(db::submissions::queue_place_for(&self.pool, student.id(), assignment_id).await?)
    }
}
