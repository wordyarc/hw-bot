use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, num};
use crate::domain::{
    EducationLevel, StudentGroup, StudentGroupId, StudyProgram, Subject, SubjectId, TeachingCourse,
    TeachingCourseId, UserId,
};

const EDUCATION_LEVEL: &str = "student_groups.education_level";
const STUDY_YEAR: &str = "student_groups.study_year";

pub async fn insert_student_group<'e, E>(executor: E, group: &StudentGroup) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into student_groups (id, code, education_level, study_year)
        values ($1, $2, $3, $4)
        "#,
        group.id().as_uuid(),
        group.code().as_str(),
        group.program().education_level().as_str(),
        i16::from(group.program().year()),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn find_student_group_by_id<'e, E>(
    executor: E,
    id: StudentGroupId,
) -> DbResult<Option<StudentGroup>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        StudentGroupRow,
        r#"
        select id, code, education_level, study_year
        from student_groups
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(StudentGroup::try_from).transpose()
}

pub async fn find_student_group_by_code<'e, E>(
    executor: E,
    code: &str,
) -> DbResult<Option<StudentGroup>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        StudentGroupRow,
        r#"
        select id, code, education_level, study_year
        from student_groups
        where code = $1
        "#,
        code,
    )
    .fetch_optional(executor)
    .await?;

    row.map(StudentGroup::try_from).transpose()
}

pub async fn list_student_groups<'e, E>(executor: E) -> DbResult<Vec<StudentGroup>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as!(
        StudentGroupRow,
        r#"
        select id, code, education_level, study_year
        from student_groups
        order by code
        "#,
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(StudentGroup::try_from)
    .collect()
}

pub async fn insert_subject<'e, E>(executor: E, subject: &Subject) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into subjects (id, name)
        values ($1, $2)
        "#,
        subject.id().as_uuid(),
        subject.name().as_str(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

/// Connection, not executor: a course without teachers is invalid, so both statements have to
/// land in the caller's transaction.
pub async fn insert_teaching_course(
    conn: &mut PgConnection,
    course: &TeachingCourse,
) -> DbResult<()> {
    sqlx::query!(
        r#"
        insert into teaching_courses (id, subject_id, student_group_id, created_at)
        values ($1, $2, $3, $4)
        "#,
        course.id().as_uuid(),
        course.subject_id().as_uuid(),
        course.student_group_id().as_uuid(),
        course.created_at(),
    )
    .execute(&mut *conn)
    .await?;

    let teacher_ids: Vec<Uuid> = course.teacher_ids().iter().map(|id| id.as_uuid()).collect();

    sqlx::query!(
        r#"
        insert into teaching_course_teachers (teaching_course_id, teacher_id)
        select $1, teacher_id
        from unnest($2::uuid[]) as teachers (teacher_id)
        "#,
        course.id().as_uuid(),
        &teacher_ids,
    )
    .execute(conn)
    .await?;

    Ok(())
}

pub async fn find_teaching_course_by_id<'e, E>(
    executor: E,
    id: TeachingCourseId,
) -> DbResult<Option<TeachingCourse>>
where
    E: PgExecutor<'e>,
{
    // Left join, а не inner: курс без преподавателей должен всплыть как испорченное состояние,
    // а не притвориться отсутствующим.
    let row = sqlx::query_as!(
        TeachingCourseRow,
        r#"
        select
            teaching_courses.id,
            teaching_courses.subject_id,
            teaching_courses.student_group_id,
            teaching_courses.created_at,
            array_remove(array_agg(teaching_course_teachers.teacher_id), null)
                as "teacher_ids!: Vec<Uuid>"
        from teaching_courses
        left join teaching_course_teachers
          on teaching_course_teachers.teaching_course_id = teaching_courses.id
        where teaching_courses.id = $1
        group by teaching_courses.id
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(TeachingCourse::try_from).transpose()
}

#[derive(Debug)]
struct StudentGroupRow {
    id: Uuid,
    code: String,
    education_level: String,
    study_year: i16,
}

impl TryFrom<StudentGroupRow> for StudentGroup {
    type Error = DbError;

    fn try_from(row: StudentGroupRow) -> Result<Self, Self::Error> {
        let education_level = EducationLevel::parse_str(&row.education_level)
            .ok_or_else(|| DbError::invalid_value(EDUCATION_LEVEL, &row.education_level))?;
        let study_year: u8 = num(STUDY_YEAR, row.study_year)?;

        Ok(StudentGroup::new(
            StudentGroupId::from_uuid(row.id),
            row.code,
            StudyProgram::new(education_level, study_year)?,
        )?)
    }
}

#[derive(Debug)]
struct TeachingCourseRow {
    id: Uuid,
    subject_id: Uuid,
    student_group_id: Uuid,
    created_at: DateTime<Utc>,
    teacher_ids: Vec<Uuid>,
}

impl TryFrom<TeachingCourseRow> for TeachingCourse {
    type Error = DbError;

    fn try_from(row: TeachingCourseRow) -> Result<Self, Self::Error> {
        let teacher_ids: HashSet<UserId> =
            row.teacher_ids.into_iter().map(UserId::from_uuid).collect();

        Ok(TeachingCourse::restore(
            TeachingCourseId::from_uuid(row.id),
            SubjectId::from_uuid(row.subject_id),
            StudentGroupId::from_uuid(row.student_group_id),
            teacher_ids,
            row.created_at,
        )?)
    }
}
