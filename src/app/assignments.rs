use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::{AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DomainError, DomainResult,
    StudentAssignmentProgress, Submission, SubmissionHistory, TeachingCourse, TeachingCourseId,
    TelegramUserId, User, UserId,
};

#[derive(Clone, Debug)]
pub struct CreateAssignment {
    pub teaching_course_id: TeachingCourseId,
    pub actor_telegram_user_id: TelegramUserId,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ChangeDeadline {
    pub assignment_id: AssignmentId,
    pub actor_telegram_user_id: TelegramUserId,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct GrantExtraAttempts {
    pub assignment_id: AssignmentId,
    pub student_id: UserId,
    pub actor_telegram_user_id: TelegramUserId,
    pub amount: u16,
}

/// Задание глазами студента
#[derive(Clone, Debug)]
pub struct StudentAssignment {
    pub assignment: Assignment,
    pub history: SubmissionHistory,
}

#[derive(Clone, Debug)]
pub struct StudentAssignmentDetails {
    pub assignment: StudentAssignment,
    pub submissions: Vec<Submission>,
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

        let author = db::users::find_by_telegram_user_id(&mut *tx, command.actor_telegram_user_id)
            .await?
            .required("teacher")?;

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
        actor_telegram_user_id: TelegramUserId,
        published_at: DateTime<Utc>,
    ) -> AppResult<Assignment> {
        self.change(
            assignment_id,
            actor_telegram_user_id,
            |assignment, course, actor| assignment.publish(course, actor, published_at),
        )
        .await
    }

    pub async fn archive(
        &self,
        assignment_id: AssignmentId,
        actor_telegram_user_id: TelegramUserId,
    ) -> AppResult<Assignment> {
        self.change(assignment_id, actor_telegram_user_id, Assignment::archive)
            .await
    }

    pub async fn change_deadline(&self, command: ChangeDeadline) -> AppResult<Assignment> {
        self.change(
            command.assignment_id,
            command.actor_telegram_user_id,
            |assignment, course, actor| {
                assignment.change_deadline(course, actor, command.deadline_at)
            },
        )
        .await
    }

    pub async fn grant_extra_attempts(
        &self,
        command: GrantExtraAttempts,
    ) -> AppResult<StudentAssignmentProgress> {
        let mut tx = self.pool.begin().await?;

        let actor = db::users::find_by_telegram_user_id(&mut *tx, command.actor_telegram_user_id)
            .await?
            .required("teacher")?;

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
    pub async fn list_for_student(
        &self,
        actor_telegram_user_id: TelegramUserId,
    ) -> AppResult<Vec<StudentAssignment>> {
        let student = self.student(actor_telegram_user_id).await?;
        let group_id = student.student_group_id().required("student group")?;

        let assignments = db::assignments::list_published_for_group(&self.pool, group_id).await?;
        let ids: Vec<_> = assignments
            .iter()
            .map(|assignment| assignment.id().as_uuid())
            .collect();
        let mut histories = db::submissions::load_histories(&self.pool, &ids, student.id()).await?;

        Ok(assignments
            .into_iter()
            .map(|assignment| StudentAssignment {
                history: histories.remove(&assignment.id()).unwrap_or_default(),
                assignment,
            })
            .collect())
    }

    pub async fn find_for_student(
        &self,
        actor_telegram_user_id: TelegramUserId,
        assignment_id: AssignmentId,
    ) -> AppResult<StudentAssignmentDetails> {
        let student = self.student(actor_telegram_user_id).await?;

        let assignment = db::assignments::find_by_id(&self.pool, assignment_id)
            .await?
            .required("assignment")?;

        // Черновика для студента не существует, поэтому 404, а не 403: знать о нём он не должен.
        if assignment.status() == AssignmentStatus::Draft {
            return Err(AppError::NotFound {
                resource: "assignment",
            });
        }

        if student.student_group_id() != Some(assignment.student_group_id()) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup.into());
        }

        let history =
            db::submissions::load_history(&self.pool, assignment.id(), student.id()).await?;
        let submissions =
            db::submissions::list_for_student(&self.pool, assignment.id(), student.id()).await?;

        Ok(StudentAssignmentDetails {
            assignment: StudentAssignment {
                assignment,
                history,
            },
            submissions,
        })
    }

    /// Экран курса у преподавателя: черновики и архив тоже его, в отличие от студенческого списка.
    pub async fn list_for_course(
        &self,
        actor_telegram_user_id: TelegramUserId,
        teaching_course_id: TeachingCourseId,
    ) -> AppResult<Vec<Assignment>> {
        let actor = db::users::find_by_telegram_user_id(&self.pool, actor_telegram_user_id)
            .await?
            .required("teacher")?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&self.pool, teaching_course_id)
                .await?
                .required("teaching course")?;

        if !teaching_course.can_be_managed_by(&actor) {
            return Err(DomainError::PermissionDenied {
                action: "list course assignments",
            }
            .into());
        }

        Ok(db::assignments::list_for_course(&self.pool, teaching_course_id).await?)
    }

    async fn student(&self, telegram_user_id: TelegramUserId) -> AppResult<User> {
        let student = db::users::find_by_telegram_user_id(&self.pool, telegram_user_id)
            .await?
            .required("student")?;

        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "list student assignments",
            }
            .into());
        }

        Ok(student)
    }

    /// Общая обвязка всех правок задания: у публикации, архивации и сдвига дедлайна отличается
    /// только доменный вызов, а загрузка актора, задания и курса одна и та же.
    async fn change<F>(
        &self,
        assignment_id: AssignmentId,
        actor_telegram_user_id: TelegramUserId,
        apply: F,
    ) -> AppResult<Assignment>
    where
        F: FnOnce(&mut Assignment, &TeachingCourse, &User) -> DomainResult<()>,
    {
        let mut tx = self.pool.begin().await?;

        let actor = db::users::find_by_telegram_user_id(&mut *tx, actor_telegram_user_id)
            .await?
            .required("teacher")?;

        let mut assignment = db::assignments::find_by_id(&mut *tx, assignment_id)
            .await?
            .required("assignment")?;

        let teaching_course =
            db::academic::find_teaching_course_by_id(&mut *tx, assignment.teaching_course_id())
                .await?
                .required("teaching course")?;

        let previous_status = assignment.status();
        apply(&mut assignment, &teaching_course, &actor)?;

        db::assignments::update(&mut *tx, &assignment, previous_status).await?;

        tx.commit().await?;

        Ok(assignment)
    }
}
