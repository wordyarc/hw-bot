use sqlx::PgPool;

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult};
use crate::db;
use crate::db::academic::TeachingCourseCard;
use crate::domain::{DomainError, StudentGroup, Subject, TeachingCourse};

#[derive(Clone, Debug)]
pub struct TeachingCourseSummary {
    pub course: TeachingCourse,
    pub subject: Subject,
    pub group: StudentGroup,
}

impl From<TeachingCourseCard> for TeachingCourseSummary {
    fn from(card: TeachingCourseCard) -> Self {
        Self {
            course: card.course,
            subject: card.subject,
            group: card.group,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CourseService {
    pool: PgPool,
}

impl CourseService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_for_teacher(&self, actor: Actor) -> AppResult<Vec<TeachingCourseSummary>> {
        let actor = load_actor(&self.pool, actor, "teacher").await?;

        if !actor.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "list teaching courses",
            }
            .into());
        }

        Ok(
            db::academic::list_teaching_courses_for_teacher(&self.pool, &actor)
                .await?
                .into_iter()
                .map(TeachingCourseSummary::from)
                .collect(),
        )
    }
}
