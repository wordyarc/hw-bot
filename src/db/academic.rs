use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::DbResult;
use crate::domain::{StudentGroup, Subject, TeachingCourse};

#[derive(Clone, Debug, Default)]
pub struct StudentGroupRepository;

impl StudentGroupRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn insert<'e, E>(&self, executor: E, group: &StudentGroup) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query(
            r#"
            insert into student_groups (id, code, education_level, study_year)
            values ($1, $2, $3, $4)
            "#,
        )
        .bind(group.id().as_uuid())
        .bind(group.code().as_str())
        .bind(group.program().education_level().as_str())
        .bind(i16::from(group.program().year()))
        .execute(executor)
        .await?;

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubjectRepository;

impl SubjectRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn insert<'e, E>(&self, executor: E, subject: &Subject) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query(
            r#"
            insert into subjects (id, name)
            values ($1, $2)
            "#,
        )
        .bind(subject.id().as_uuid())
        .bind(subject.name().as_str())
        .execute(executor)
        .await?;

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct TeachingCourseRepository;

impl TeachingCourseRepository {
    pub fn new() -> Self {
        Self
    }

    /// Connection, not executor: a course without teachers is invalid, so both statements have to
    /// land in the caller's transaction.
    pub async fn insert(&self, conn: &mut PgConnection, course: &TeachingCourse) -> DbResult<()> {
        sqlx::query(
            r#"
            insert into teaching_courses (id, subject_id, student_group_id, created_at)
            values ($1, $2, $3, $4)
            "#,
        )
        .bind(course.id().as_uuid())
        .bind(course.subject_id().as_uuid())
        .bind(course.student_group_id().as_uuid())
        .bind(course.created_at())
        .execute(&mut *conn)
        .await?;

        let teacher_ids: Vec<Uuid> = course.teacher_ids().iter().map(|id| id.as_uuid()).collect();

        sqlx::query(
            r#"
            insert into teaching_course_teachers (teaching_course_id, teacher_id)
            select $1, teacher_id
            from unnest($2::uuid[]) as teachers (teacher_id)
            "#,
        )
        .bind(course.id().as_uuid())
        .bind(&teacher_ids)
        .execute(conn)
        .await?;

        Ok(())
    }
}
