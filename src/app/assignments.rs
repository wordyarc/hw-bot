use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::access::{load_managing_teacher, load_student};
use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DomainError, DomainResult, Grading, LatePolicy,
    StoredFileMetadata, StudentAssignmentProgress, Subject, Submission, SubmissionHistory,
    TeachingCourse, TeachingCourseId, User, UserId,
};

#[derive(Clone, Debug)]
pub struct CreateAssignment {
    pub teaching_course_id: TeachingCourseId,
    pub actor: Actor,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ChangeDeadline {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ChangeGrading {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
}

#[derive(Clone, Debug)]
pub struct GrantExtraAttempts {
    pub assignment_id: AssignmentId,
    pub student_id: UserId,
    pub actor: Actor,
    pub amount: u16,
}

/// Задание глазами студента
#[derive(Clone, Debug)]
pub struct StudentAssignment {
    pub assignment: Assignment,
    pub subject: Subject,
    pub history: SubmissionHistory,
}

#[derive(Clone, Debug)]
pub struct StudentAssignmentDetails {
    pub assignment: StudentAssignment,
    pub files: Vec<StoredFileMetadata>,
    pub submissions: Vec<Submission>,
}

#[derive(Clone, Debug)]
pub struct AssignmentStudentSummary {
    pub student: User,
    pub history: SubmissionHistory,
}

impl From<db::users::AssignmentStudentCard> for AssignmentStudentSummary {
    fn from(card: db::users::AssignmentStudentCard) -> Self {
        Self {
            student: card.student,
            history: card.history,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AssignmentService {
    pool: PgPool,
}

impl AssignmentService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, command: CreateAssignment) -> AppResult<Assignment> {
        let mut tx = self.pool.begin().await?;

        let author = load_actor(&mut *tx, command.actor, "teacher").await?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, command.teaching_course_id)
                .await?
                .required("teaching course")?;

        let assignment = Assignment::new(
            AssignmentId::new(),
            &teaching_course,
            &author,
            command.title,
            command.body,
            command.max_attempts,
            command.grading,
            command.late_policy,
            command.created_at,
            command.deadline_at,
        )?;

        db::assignments::insert(&mut *tx, &assignment).await?;

        tx.commit().await?;

        Ok(assignment)
    }

    pub async fn publish(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
        published_at: DateTime<Utc>,
    ) -> AppResult<Assignment> {
        self.change(assignment_id, actor, |assignment, course, actor| {
            assignment.publish(course, actor, published_at)
        })
        .await
    }

    pub async fn archive(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
    ) -> AppResult<Assignment> {
        self.change(assignment_id, actor, Assignment::archive).await
    }

    pub async fn change_deadline(&self, command: ChangeDeadline) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor,
            |assignment, course, actor| {
                assignment.change_deadline(course, actor, command.deadline_at)
            },
        )
        .await
    }

    pub async fn change_grading(&self, command: ChangeGrading) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor,
            |assignment, course, actor| {
                assignment.change_grading(course, actor, command.grading, command.late_policy)
            },
        )
        .await
    }

    pub async fn grant_extra_attempts(
        &self,
        command: GrantExtraAttempts,
    ) -> AppResult<StudentAssignmentProgress> {
        let mut tx = self.pool.begin().await?;

        let actor = load_actor(&mut *tx, command.actor, "teacher").await?;

        let assignment = db::assignments::find_by_id(&mut *tx, command.assignment_id)
            .await?
            .required("assignment")?;

        let student = db::users::find_by_id(&mut *tx, command.student_id)
            .await?
            .required("student")?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, assignment.teaching_course_id())
                .await?
                .required("teaching course")?;

        let extra_attempts =
            db::attempts::lock_or_create(&mut tx, assignment.id(), student.id()).await?;
        let mut progress = StudentAssignmentProgress::new(&assignment, &student, extra_attempts)?;

        progress.grant_extra_attempts(&assignment, &teaching_course, &actor, command.amount)?;

        db::attempts::update(&mut tx, &progress).await?;
        tx.commit().await?;

        Ok(progress)
    }

    /// Главный экран студента.
    pub async fn list_for_student(&self, actor: Actor) -> AppResult<Vec<StudentAssignment>> {
        let student = self.student(actor).await?;
        let group_id = student.student_group_id().required("student group")?;

        let assignments = db::assignments::list_published_for_group(&self.pool, group_id).await?;
        self.with_student_histories(assignments, student.id()).await
    }

    pub async fn list_archived_for_student(
        &self,
        actor: Actor,
    ) -> AppResult<Vec<StudentAssignment>> {
        let student = self.student(actor).await?;
        let group_id = student.student_group_id().required("student group")?;

        let assignments = db::assignments::list_archived_for_group(&self.pool, group_id).await?;
        self.with_student_histories(assignments, student.id()).await
    }

    async fn with_student_histories(
        &self,
        cards: Vec<db::assignments::StudentAssignmentCard>,
        student_id: UserId,
    ) -> AppResult<Vec<StudentAssignment>> {
        let ids: Vec<_> = cards
            .iter()
            .map(|card| card.assignment.id().as_uuid())
            .collect();
        let mut histories = db::submissions::load_histories(&self.pool, &ids, student_id).await?;

        Ok(cards
            .into_iter()
            .map(|card| StudentAssignment {
                history: histories.remove(&card.assignment.id()).unwrap_or_default(),
                assignment: card.assignment,
                subject: card.subject,
            })
            .collect())
    }

    pub async fn find_for_student(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<StudentAssignmentDetails> {
        let (student, assignment) =
            load_visible_assignment_for_student(&self.pool, actor, assignment_id).await?;

        let history =
            db::submissions::load_history(&self.pool, assignment.id(), student.id()).await?;
        let submissions =
            db::submissions::list_for_student(&self.pool, assignment.id(), student.id()).await?;
        let files = db::files::list_for_assignment(&self.pool, assignment.id()).await?;
        let subject = db::academic::find_subject_by_id(&self.pool, assignment.subject_id())
            .await?
            .required("subject")?;

        Ok(StudentAssignmentDetails {
            assignment: StudentAssignment {
                assignment,
                subject,
                history,
            },
            files,
            submissions,
        })
    }

    pub async fn find_for_teacher(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<Assignment> {
        self.load_managed_assignment(actor, assignment_id, "view assignment")
            .await
    }

    pub async fn list_students_for_assignment(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<Vec<AssignmentStudentSummary>> {
        let assignment = self
            .load_managed_assignment(actor, assignment_id, "list assignment students")
            .await?;

        Ok(db::users::list_students_with_assignment_history(
            &self.pool,
            assignment.id(),
            assignment.student_group_id(),
        )
        .await?
        .into_iter()
        .map(AssignmentStudentSummary::from)
        .collect())
    }

    /// Экран курса у преподавателя: черновики и архив тоже его, в отличие от студенческого списка.
    pub async fn list_for_course(
        &self,
        actor: Actor,
        teaching_course_id: TeachingCourseId,
    ) -> AppResult<Vec<Assignment>> {
        load_managing_teacher(
            &self.pool,
            actor,
            teaching_course_id,
            "list course assignments",
        )
        .await?;

        Ok(db::assignments::list_for_course(&self.pool, teaching_course_id).await?)
    }

    async fn student(&self, actor: Actor) -> AppResult<User> {
        load_student(&self.pool, actor, "list student assignments").await
    }

    async fn load_managed_assignment(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        action: &'static str,
    ) -> AppResult<Assignment> {
        let assignment = db::assignments::find_by_id(&self.pool, assignment_id)
            .await?
            .required("assignment")?;
        let (actor, teaching_course) =
            load_managing_teacher(&self.pool, actor, assignment.teaching_course_id(), action)
                .await?;

        assignment.ensure_managed_by(&teaching_course, &actor, action)?;

        Ok(assignment)
    }

    /// Общая обвязка всех правок задания: у публикации, архивации и сдвига дедлайна отличается
    /// только доменный вызов, а загрузка актора, задания и курса одна и та же.
    async fn change<F>(
        &self,
        assignment_id: AssignmentId,
        actor: Actor,
        apply: F,
    ) -> AppResult<Assignment>
    where
        F: FnOnce(&mut Assignment, &TeachingCourse, &User) -> DomainResult<()>,
    {
        let mut tx = self.pool.begin().await?;

        let actor = load_actor(&mut *tx, actor, "teacher").await?;

        let stored = db::assignments::find_versioned_by_id(&mut *tx, assignment_id)
            .await?
            .required("assignment")?;
        let revision = stored.revision();
        let mut assignment = stored.into_assignment();

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, assignment.teaching_course_id())
                .await?
                .required("teaching course")?;

        apply(&mut assignment, &teaching_course, &actor)?;

        match db::assignments::update(&mut *tx, &assignment, revision).await {
            Err(db::DbError::Conflict { .. }) => {
                return Err(AppError::AssignmentChangedConcurrently);
            }
            result => result?,
        }

        tx.commit().await?;

        Ok(assignment)
    }
}

pub(crate) async fn load_visible_assignment_for_student(
    pool: &PgPool,
    actor: Actor,
    assignment_id: AssignmentId,
) -> AppResult<(User, Assignment)> {
    let student = load_student(pool, actor, "view student assignment").await?;

    let assignment = db::assignments::find_by_id(pool, assignment_id)
        .await?
        .required("assignment")?;

    // Никогда не опубликованного задания для студента не существует, поэтому 404, а не 403
    if assignment.status() == AssignmentStatus::Draft
        || (assignment.status() == AssignmentStatus::Archived
            && assignment.published_at().is_none())
    {
        return Err(AppError::NotFound {
            resource: "assignment",
        });
    }
    if student.student_group_id() != Some(assignment.student_group_id()) {
        return Err(DomainError::StudentDoesNotBelongToAssignmentGroup.into());
    }

    Ok((student, assignment))
}
