use sqlx::postgres::PgExecutor;
use sqlx::{PgConnection, PgPool};

use crate::app::users::load_actor;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    AssignmentId, DomainError, Participation, StudentGroupId, TeachingCourse, TeachingCourseId,
    User, UserId,
};
use crate::read_models::{TeachingCourseDetails, VersionedAssignment};

pub(crate) async fn load_managed_assignment(
    pool: &PgPool,
    actor: Actor,
    assignment_id: AssignmentId,
    action: &'static str,
) -> AppResult<ManagedAssignment> {
    load_managed_assignment_on(&mut *pool.acquire().await?, actor, assignment_id, action).await
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
    let (actor, course) = load_managed_course_on(
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

pub(crate) async fn load_managed_course(
    pool: &PgPool,
    actor: Actor,
    course_id: TeachingCourseId,
    action: &'static str,
) -> AppResult<(User, TeachingCourse)> {
    load_managed_course_on(&mut *pool.acquire().await?, actor, course_id, action).await
}

pub(crate) async fn load_managed_course_on(
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

pub(crate) async fn load_managed_course_details(
    pool: &PgPool,
    actor: Actor,
    course_id: TeachingCourseId,
    action: &'static str,
) -> AppResult<TeachingCourseDetails> {
    let teacher = load_actor(pool, actor, "teacher").await?;
    let details = db::academic::find_teaching_course_details(pool, course_id)
        .await?
        .required("teaching course")?;
    ensure_manages(&details.course, &teacher, action)?;

    Ok(details)
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

pub(crate) async fn load_course_student(
    connection: &mut PgConnection,
    course: &TeachingCourse,
    student_id: UserId,
) -> AppResult<(User, StudentGroupId)> {
    let (student, group_id) = db::users::find_by_id(&mut *connection, student_id)
        .await?
        .and_then(|user| {
            let group_id = user
                .student_group_id()
                .filter(|&group_id| course.includes_group(group_id))?;
            Some((user, group_id))
        })
        .required("student")?;

    match db::enrollments::participation(connection, course.id(), student.id()).await? {
        Participation::Participant => Ok((student, group_id)),
        Participation::Outsider => Err(AppError::NotFound {
            resource: "student",
        }),
    }
}

pub(crate) async fn load_student(
    executor: impl PgExecutor<'_>,
    actor: Actor,
    action: &'static str,
) -> AppResult<User> {
    let student = load_actor(executor, actor, "student").await?;
    if !student.role().can_submit_homework() {
        return Err(DomainError::PermissionDenied { action }.into());
    }

    Ok(student)
}
