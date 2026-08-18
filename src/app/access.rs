use sqlx::postgres::PgExecutor;

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{DomainError, TeachingCourse, TeachingCourseId, User};

/// Преподаватель вместе с курсом, которым он вправе управлять.
pub(crate) async fn load_managing_teacher<'e, E>(
    executor: E,
    actor: Actor,
    course_id: TeachingCourseId,
    action: &'static str,
) -> AppResult<(User, TeachingCourse)>
where
    E: PgExecutor<'e> + Copy,
{
    let actor = load_actor(executor, actor, "teacher").await?;
    let course = db::academic::find_teaching_course_by_id(executor, course_id)
        .await?
        .required("teaching course")?;

    ensure_manages(&course, &actor, action)?;

    Ok((actor, course))
}

pub(crate) fn ensure_manages(
    course: &TeachingCourse,
    actor: &User,
    action: &'static str,
) -> AppResult<()> {
    if !course.can_be_managed_by(actor) {
        return Err(DomainError::PermissionDenied { action }.into());
    }

    Ok(())
}

pub(crate) async fn load_student<'e, E>(
    executor: E,
    actor: Actor,
    action: &'static str,
) -> AppResult<User>
where
    E: PgExecutor<'e>,
{
    let student = load_actor(executor, actor, "student").await?;
    if !student.role().can_submit_homework() {
        return Err(DomainError::PermissionDenied { action }.into());
    }

    Ok(student)
}
