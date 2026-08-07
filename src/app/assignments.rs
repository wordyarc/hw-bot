use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::app::{AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, DomainResult, TeachingCourse, TeachingCourseId, TelegramUserId, User,
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
