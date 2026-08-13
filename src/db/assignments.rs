use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    Assignment, AssignmentId, AssignmentParts, AssignmentStatus, StudentGroupId, Subject,
    SubjectId, TeachingCourseId, UserId,
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
        checked_num_cast::<_, i16>(MAX_ATTEMPTS, assignment.max_attempts())?,
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

pub async fn update<'e, E>(
    executor: E,
    assignment: &Assignment,
    expected_revision: i64,
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
            published_at = $4,
            revision = revision + 1
        where id = $1
          and revision = $5
        "#,
        assignment.id().as_uuid(),
        assignment.status().as_str(),
        assignment.deadline_at(),
        assignment.published_at(),
        expected_revision,
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

pub async fn advance_revision<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    expected_revision: i64,
) -> DbResult<i64>
where
    E: PgExecutor<'e>,
{
    sqlx::query_scalar!(
        r#"
        update assignments
        set revision = revision + 1
        where id = $1
          and revision = $2
        returning revision
        "#,
        assignment_id.as_uuid(),
        expected_revision,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::Conflict {
        message: "assignment was changed by a concurrent request",
    })
}

#[derive(Debug)]
pub struct VersionedAssignment {
    assignment: Assignment,
    revision: i64,
}

impl VersionedAssignment {
    pub fn into_assignment(self) -> Assignment {
        self.assignment
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }
}

pub async fn find_versioned_by_id<'e, E>(
    executor: E,
    id: AssignmentId,
) -> DbResult<Option<VersionedAssignment>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        VersionedAssignmentRow,
        r#"
        select
            id, teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, created_at, deadline_at, published_at, revision
        from assignments
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(VersionedAssignment::try_from).transpose()
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

/// Список студента идёт по предметам: у группы их несколько, и без названия предмета задания
/// сливаются в одну кучу. Внутри предмета порядок прежний - по сроку.
pub async fn list_published_for_group<'e, E>(
    executor: E,
    student_group_id: StudentGroupId,
) -> DbResult<Vec<StudentAssignmentCard>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as!(
        StudentAssignmentCardRow,
        r#"
        select
            assignments.id, assignments.teaching_course_id, assignments.subject_id,
            assignments.student_group_id, assignments.title, assignments.body,
            assignments.max_attempts, assignments.status, assignments.created_by,
            assignments.created_at, assignments.deadline_at, assignments.published_at,
            subjects.name as subject_name
        from assignments
        join subjects
          on subjects.id = assignments.subject_id
        where assignments.student_group_id = $1
          and assignments.status = 'published'
        order by subjects.name, assignments.deadline_at, assignments.id
        "#,
        student_group_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(StudentAssignmentCard::try_from)
    .collect()
}

pub async fn list_archived_for_group<'e, E>(
    executor: E,
    student_group_id: StudentGroupId,
) -> DbResult<Vec<StudentAssignmentCard>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as!(
        StudentAssignmentCardRow,
        r#"
        select
            assignments.id, assignments.teaching_course_id, assignments.subject_id,
            assignments.student_group_id, assignments.title, assignments.body,
            assignments.max_attempts, assignments.status, assignments.created_by,
            assignments.created_at, assignments.deadline_at, assignments.published_at,
            subjects.name as subject_name
        from assignments
        join subjects
          on subjects.id = assignments.subject_id
        where assignments.student_group_id = $1
          and assignments.status = 'archived'
          and assignments.published_at is not null
        order by subjects.name, assignments.published_at desc, assignments.id
        "#,
        student_group_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(StudentAssignmentCard::try_from)
    .collect()
}

/// Всё, что есть у курса, включая черновики и архив - экран преподавателя.
pub async fn list_for_course<'e, E>(
    executor: E,
    teaching_course_id: TeachingCourseId,
) -> DbResult<Vec<Assignment>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as!(
        AssignmentRow,
        r#"
        select
            id, teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, created_at, deadline_at, published_at
        from assignments
        where teaching_course_id = $1
        order by created_at desc, id
        "#,
        teaching_course_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Assignment::try_from)
    .collect()
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

#[derive(Debug)]
struct VersionedAssignmentRow {
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
    revision: i64,
}

impl TryFrom<VersionedAssignmentRow> for VersionedAssignment {
    type Error = DbError;

    fn try_from(row: VersionedAssignmentRow) -> Result<Self, Self::Error> {
        let revision = row.revision;
        let assignment = Assignment::try_from(AssignmentRow {
            id: row.id,
            teaching_course_id: row.teaching_course_id,
            subject_id: row.subject_id,
            student_group_id: row.student_group_id,
            title: row.title,
            body: row.body,
            max_attempts: row.max_attempts,
            status: row.status,
            created_by: row.created_by,
            created_at: row.created_at,
            deadline_at: row.deadline_at,
            published_at: row.published_at,
        })?;

        Ok(Self {
            assignment,
            revision,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StudentAssignmentCard {
    pub assignment: Assignment,
    pub subject: Subject,
}

#[derive(Debug)]
struct StudentAssignmentCardRow {
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
    subject_name: String,
}

impl TryFrom<StudentAssignmentCardRow> for StudentAssignmentCard {
    type Error = DbError;

    fn try_from(row: StudentAssignmentCardRow) -> Result<Self, Self::Error> {
        let subject = Subject::new(SubjectId::from_uuid(row.subject_id), row.subject_name)?;
        let assignment = Assignment::try_from(AssignmentRow {
            id: row.id,
            teaching_course_id: row.teaching_course_id,
            subject_id: row.subject_id,
            student_group_id: row.student_group_id,
            title: row.title,
            body: row.body,
            max_attempts: row.max_attempts,
            status: row.status,
            created_by: row.created_by,
            created_at: row.created_at,
            deadline_at: row.deadline_at,
            published_at: row.published_at,
        })?;

        Ok(Self {
            assignment,
            subject,
        })
    }
}

impl TryFrom<AssignmentRow> for Assignment {
    type Error = DbError;

    fn try_from(row: AssignmentRow) -> Result<Self, Self::Error> {
        let max_attempts: u16 = checked_num_cast(MAX_ATTEMPTS, row.max_attempts)?;
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
