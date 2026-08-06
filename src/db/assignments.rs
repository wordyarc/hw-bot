use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, try_u16_from_i16};
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, StudentGroupId, SubjectId, TeachingCourseId, UserId,
};

#[derive(Clone, Debug, Default)]
pub struct AssignmentRepository;

impl AssignmentRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: AssignmentId,
    ) -> DbResult<Option<Assignment>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, AssignmentRow>(
            r#"
            select
                id,
                teaching_course_id,
                subject_id,
                student_group_id,
                title,
                body,
                max_attempts,
                status,
                created_by,
                created_at,
                deadline_at,
                published_at
            from assignments
            where id = $1
            "#,
        )
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
        let status = assignment_status_from_db(&row.status)?;

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

fn assignment_status_from_db(value: &str) -> DbResult<AssignmentStatus> {
    match value {
        "draft" => Ok(AssignmentStatus::Draft),
        "published" => Ok(AssignmentStatus::Published),
        "archived" => Ok(AssignmentStatus::Archived),
        other => Err(DbError::InvalidDatabaseValue {
            field: "assignments.status",
            value: other.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_assignment_status_from_database_values() {
        assert_eq!(
            assignment_status_from_db("draft").unwrap(),
            AssignmentStatus::Draft
        );
        assert_eq!(
            assignment_status_from_db("published").unwrap(),
            AssignmentStatus::Published
        );
        assert_eq!(
            assignment_status_from_db("archived").unwrap(),
            AssignmentStatus::Archived
        );
        assert!(matches!(
            assignment_status_from_db("unknown"),
            Err(DbError::InvalidDatabaseValue {
                field: "assignments.status",
                ..
            })
        ));
    }
}
