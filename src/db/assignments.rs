use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, to_db_smallint, try_u16_from_i16};
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, StudentGroupId, SubjectId, TeachingCourseId, UserId,
};

macro_rules! columns {
    () => {
        "id, teaching_course_id, subject_id, student_group_id, title, body, max_attempts, status, \
         created_by, created_at, deadline_at, published_at"
    };
}

#[derive(Clone, Debug, Default)]
pub struct AssignmentRepository;

impl AssignmentRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn insert<'e, E>(&self, executor: E, assignment: &Assignment) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query(concat!(
            "insert into assignments (",
            columns!(),
            ") values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)"
        ))
        .bind(assignment.id().as_uuid())
        .bind(assignment.teaching_course_id().as_uuid())
        .bind(assignment.subject_id().as_uuid())
        .bind(assignment.student_group_id().as_uuid())
        .bind(assignment.title().as_str())
        .bind(assignment.body().as_str())
        .bind(to_db_smallint(
            "assignment.max_attempts",
            assignment.max_attempts(),
        )?)
        .bind(assignment.status().as_db_str())
        .bind(assignment.created_by().as_uuid())
        .bind(assignment.created_at())
        .bind(assignment.deadline_at())
        .bind(assignment.published_at())
        .execute(executor)
        .await?;

        Ok(())
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: AssignmentId,
    ) -> DbResult<Option<Assignment>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, AssignmentRow>(concat!(
            "select ",
            columns!(),
            " from assignments where id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(executor)
        .await?;

        row.map(Assignment::try_from).transpose()
    }
}

#[derive(Debug, FromRow)]
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
        let max_attempts = try_u16_from_i16("assignments.max_attempts", row.max_attempts)?;
        let status = AssignmentStatus::from_db_str(&row.status)?;

        Ok(Assignment::restore(
            AssignmentId::from_uuid(row.id),
            TeachingCourseId::from_uuid(row.teaching_course_id),
            SubjectId::from_uuid(row.subject_id),
            StudentGroupId::from_uuid(row.student_group_id),
            row.title,
            row.body,
            max_attempts,
            status,
            UserId::from_uuid(row.created_by),
            row.created_at,
            row.deadline_at,
            row.published_at,
        )?)
    }
}
