use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::access::load_student;
use crate::app::assignments::load_student_assignment;
use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required, StudentAssignment};
use crate::db;
use crate::domain::{
    Acceptance, AssignmentId, MakeupBalance, MakeupRequest, Resubmission, Submission, SubmissionId,
    SubmissionKind, Submitter,
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
pub struct WithdrawSubmission {
    pub submission_id: SubmissionId,
    pub actor: Actor,
    pub withdrawn_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct WithdrawalPreview {
    pub submission: Submission,
    pub assignment: StudentAssignment,
}

impl WithdrawalPreview {
    pub fn resubmission_at(&self, at: DateTime<Utc>) -> Resubmission {
        self.submission.resubmission_at(
            &self.assignment.assignment,
            self.assignment.history,
            self.assignment.makeup_pool,
            at,
        )
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

    pub async fn submit_homework(&self, command: SubmitHomework) -> AppResult<SubmittedHomework> {
        let mut tx = self.pool.begin().await?;

        let student = db::users::find_for_key_share(&mut *tx, command.actor.user_id())
            .await?
            .required("student")?;

        let assignment = db::assignments::find_by_id(&mut *tx, command.assignment_id)
            .await?
            .required("assignment")?;

        let history = db::submissions::load_histories(&mut *tx, &[assignment.id()], student.id())
            .await?
            .remove(&assignment.id())
            .unwrap_or_default();

        let course_id = assignment.teaching_course_id();
        let mut makeup = MakeupRequest {
            course: db::makeup::find_course(&mut *tx, course_id).await?,
            consent: command.allow_makeup,
            balance: MakeupBalance::default(),
        };
        let participation =
            db::enrollments::participation(&mut *tx, course_id, student.id()).await?;
        let submitter = Submitter {
            student: &student,
            participation,
        };
        let acceptance = Submission::admit(
            &assignment,
            submitter,
            history,
            command.submitted_at,
            makeup,
        )?;
        if acceptance != Acceptance::Regular {
            db::makeup::lock_or_create_pool(&mut tx, course_id, student.id()).await?;
            let pool = db::makeup::pool_for_student(&mut *tx, course_id, student.id()).await?;
            makeup.course = pool.map(|pool| pool.course);
            makeup.balance = pool.map(|pool| pool.balance).unwrap_or_default();
        }

        let submission = Submission::submit(
            SubmissionId::new(),
            &assignment,
            submitter,
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

    pub async fn find_for_withdrawal(
        &self,
        actor: Actor,
        submission_id: SubmissionId,
    ) -> AppResult<WithdrawalPreview> {
        let author = load_actor(&self.pool, actor, "student").await?;
        let submission = db::submissions::find_by_id(&self.pool, submission_id)
            .await?
            .required("submission")?;
        submission.ensure_authored_by(&author, "prepare submission withdrawal")?;
        let assignment = db::assignments::find_by_id(&self.pool, submission.assignment_id())
            .await?
            .required("assignment")?;

        Ok(WithdrawalPreview {
            assignment: load_student_assignment(&self.pool, &author, assignment).await?,
            submission,
        })
    }

    pub async fn withdraw(&self, command: WithdrawSubmission) -> AppResult<Submission> {
        let mut connection = self.pool.acquire().await?;
        let author = load_actor(&mut *connection, command.actor, "student").await?;
        let mut submission = db::submissions::find_by_id(&mut *connection, command.submission_id)
            .await?
            .required("submission")?;

        submission.withdraw(&author, command.withdrawn_at)?;
        db::submissions::withdraw(&mut connection, &submission).await?;

        Ok(submission)
    }

    pub async fn queue_places(
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
        Ok(db::submissions::queue_place_for(&self.pool, assignment_id, student.id()).await?)
    }
}
