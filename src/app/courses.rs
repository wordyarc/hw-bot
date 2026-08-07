use sqlx::PgPool;

use crate::app::{AppResult, Required};
use crate::db;
use crate::db::academic::TeachingCourseCard;
use crate::domain::{DomainError, TelegramUserId};

#[derive(Clone, Debug)]
pub struct CourseService {
    pool: PgPool,
}

impl CourseService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_for_teacher(
        &self,
        actor_telegram_user_id: TelegramUserId,
    ) -> AppResult<Vec<TeachingCourseCard>> {
        let actor = db::users::find_by_telegram_user_id(&self.pool, actor_telegram_user_id)
            .await?
            .required("teacher")?;

        if !actor.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "list teaching courses",
            }
            .into());
        }

        Ok(db::academic::list_teaching_courses_for_teacher(&self.pool, &actor).await?)
    }
}
