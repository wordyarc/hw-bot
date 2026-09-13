use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{DomainError, TeachingCourse, TeachingCourseId, User};

pub(crate) async fn load_managing_teacher(
    pool: &PgPool,
    actor: Actor,
    course_id: TeachingCourseId,
    action: &'static str,
) -> AppResult<(User, TeachingCourse)> {
    load_managing_teacher_on(&mut *pool.acquire().await?, actor, course_id, action).await
}

pub(crate) async fn load_managing_teacher_on(
    connection: &mut PgConnection,
    actor: Actor,
    course_id: TeachingCourseId,
    action: &'static str,
) -> AppResult<(User, TeachingCourse)> {
    let actor = load_actor(&mut *connection, actor, "teacher").await?;
    let course = db::academic::find_teaching_course_by_id(&mut *connection, course_id)
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
