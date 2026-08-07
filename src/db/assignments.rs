use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, num};
use crate::domain::{
    Assignment, AssignmentId, AssignmentParts, AssignmentStatus, StudentGroupId, SubjectId,
    TeachingCourseId, UserId,
};

const MAX_ATTEMPTS: &str = "assignments.max_attempts";
const STATUS: &str = "assignments.status";

pub async fn insert<'e, E>(executor: E, assignment: &Assignment) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into assignments (
            id, teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, created_at, deadline_at, published_at
        )
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
        assignment.id().as_uuid(),
        assignment.teaching_course_id().as_uuid(),
        assignment.subject_id().as_uuid(),
        assignment.student_group_id().as_uuid(),
        assignment.title().as_str(),
        assignment.body().as_str(),
        num::<_, i16>(MAX_ATTEMPTS, assignment.max_attempts())?,
        assignment.status().as_str(),
        assignment.created_by().as_uuid(),
        assignment.created_at(),
        assignment.deadline_at(),
        assignment.published_at(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Update условный по прежнему статусу: одновременные publish и archive иначе молча затирают друг
/// друга, и проигравший получает 200 на переход, которого в БД уже нет.
pub async fn update<'e, E>(
    executor: E,
    assignment: &Assignment,
    previous_status: AssignmentStatus,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let result = sqlx::query!(
        r#"
        update assignments
        set
            status = $2,
            deadline_at = $3,
            published_at = $4
        where id = $1
          and status = $5
        "#,
        assignment.id().as_uuid(),
        assignment.status().as_str(),
        assignment.deadline_at(),
        assignment.published_at(),
        previous_status.as_str(),
    )
    .execute(executor)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::Conflict {
            message: "assignment was changed by a concurrent request",
        });
    }

    Ok(())
}

pub async fn find_by_id<'e, E>(executor: E, id: AssignmentId) -> DbResult<Option<Assignment>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        AssignmentRow,
        r#"
        select
            id, teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, created_at, deadline_at, published_at
        from assignments
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(Assignment::try_from).transpose()
}

#[derive(Debug)]
struct AssignmentRow {
    id: Uuid,
    teaching_course_id: Uuid,
    subject_id: Uuid,
    student_group_id: Uuid,
    title: String,
    body: String,
    max_attempts: i16,
    status: String,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

impl TryFrom<AssignmentRow> for Assignment {
    type Error = DbError;

    fn try_from(row: AssignmentRow) -> Result<Self, Self::Error> {
        let max_attempts: u16 = num(MAX_ATTEMPTS, row.max_attempts)?;
        let status = AssignmentStatus::parse_str(&row.status)
            .ok_or_else(|| DbError::invalid_value(STATUS, &row.status))?;

        Ok(Assignment::restore(AssignmentParts {
            id: AssignmentId::from_uuid(row.id),
            teaching_course_id: TeachingCourseId::from_uuid(row.teaching_course_id),
            subject_id: SubjectId::from_uuid(row.subject_id),
            student_group_id: StudentGroupId::from_uuid(row.student_group_id),
            title: row.title,
            body: row.body,
            max_attempts,
            status,
            created_by: UserId::from_uuid(row.created_by),
            created_at: row.created_at,
            deadline_at: row.deadline_at,
            published_at: row.published_at,
        })?)
    }
}
