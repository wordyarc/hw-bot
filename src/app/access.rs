use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::users::load_actor;
use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{
    AssignmentId, DomainError, StudentGroupId, TeachingCourse, TeachingCourseId, User, UserId,
};
use crate::read_models::VersionedAssignment;

pub(crate) async fn load_managed_versioned_assignment(
    pool: &PgPool,
    actor: Actor,
    assignment_id: AssignmentId,
    action: &'static str,
) -> AppResult<VersionedAssignment> {
    load_managed_versioned_assignment_on(&mut *pool.acquire().await?, actor, assignment_id, action)
        .await
}

pub(crate) async fn load_managed_versioned_assignment_on(
    connection: &mut PgConnection,
    actor: Actor,
    assignment_id: AssignmentId,
    action: &'static str,
) -> AppResult<VersionedAssignment> {
    Ok(
        load_managed_assignment_on(connection, actor, assignment_id, action)
            .await?
            .stored,
    )
}

pub(crate) struct ManagedAssignment {
    pub actor: User,
    pub course: TeachingCourse,
    pub stored: VersionedAssignment,
}

pub(crate) async fn load_managed_assignment_on(
    connection: &mut PgConnection,
    actor: Actor,
    assignment_id: AssignmentId,
    action: &'static str,
) -> AppResult<ManagedAssignment> {
    let stored = db::assignments::find_versioned_by_id(&mut *connection, assignment_id)
        .await?
        .required("assignment")?;
    let (actor, course) = load_managing_teacher_on(
        connection,
        actor,
        stored.assignment.teaching_course_id(),
        action,
    )
    .await?;
    Ok(ManagedAssignment {
        actor,
        course,
        stored,
    })
}

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

pub(crate) async fn load_course_student<'e, E>(
    executor: E,
    course: &TeachingCourse,
    student_id: UserId,
) -> AppResult<(User, StudentGroupId)>
where
    E: PgExecutor<'e>,
{
    db::users::find_by_id(executor, student_id)
        .await?
        .and_then(|user| {
            let group_id = user
                .student_group_id()
                .filter(|&group_id| course.includes_group(group_id))?;
            Some((user, group_id))
        })
        .required("student")
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
