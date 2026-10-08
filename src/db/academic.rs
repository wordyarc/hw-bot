use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    EducationLevel, StudentGroup, StudentGroupId, StudyProgram, Subject, SubjectId, TeachingCourse,
    TeachingCourseId, User, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::TeachingCourseSummary;

const EDUCATION_LEVEL: &str = "student_groups.education_level";
const STUDY_YEAR: &str = "student_groups.study_year";

pub async fn insert_student_group<'e, E>(executor: E, group: &StudentGroup) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    // Создаём инвайт при добавлении группы, чтобы открытие экрана ссылки оставалось чтением.
    sqlx::query!(
        r#"
        with created as (
            insert into student_groups (id, code, education_level, study_year)
            values ($1, $2, $3, $4)
            returning id
        )
        insert into student_group_invites (student_group_id, generation)
        select id, 1 from created
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

pub async fn find_subject_by_id<'e, E>(executor: E, id: SubjectId) -> DbResult<Option<Subject>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query!(
        r#"
        select id, name
        from subjects
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(|row| Subject::new(SubjectId::from_uuid(row.id), row.name))
        .transpose()
        .map_err(DbError::from)
}

pub async fn find_subject_sandbox<'e, E>(executor: E, id: SubjectId) -> DbResult<Option<bool>>
where
    E: PgExecutor<'e>,
{
    Ok(sqlx::query_scalar!(
        r#"
        select sandbox
        from subjects
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?)
}

/// Вызывающий обязан объединить оба INSERT транзакцией: нельзя оставить курс без преподавателей.
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
    // Через left join обнаружится курс без преподавателей. Inner join скрыл бы повреждённую запись.
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

pub(crate) async fn list_teaching_courses_for_teacher<'e, E>(
    executor: E,
    teacher: &User,
    page: PageRequest,
) -> DbResult<Vec<TeachingCourseSummary>>
where
    E: PgExecutor<'e>,
{
    let sees_every_course = teacher.role().can_manage_all_teaching_courses();

    sqlx::query_as!(
        TeachingCourseCardRow,
        r#"
        select
            teaching_courses.id,
            teaching_courses.subject_id,
            teaching_courses.student_group_id,
            teaching_courses.created_at,
            array_remove(array_agg(teaching_course_teachers.teacher_id), null)
                as "teacher_ids!: Vec<Uuid>",
            subjects.name as subject_name,
            student_groups.code as group_code,
            student_groups.education_level,
            student_groups.study_year
        from teaching_courses
        join subjects
          on subjects.id = teaching_courses.subject_id
        join student_groups
          on student_groups.id = teaching_courses.student_group_id
        left join teaching_course_teachers
          on teaching_course_teachers.teaching_course_id = teaching_courses.id
        where $1 or exists (
            select 1
            from teaching_course_teachers mine
            where mine.teaching_course_id = teaching_courses.id
              and mine.teacher_id = $2
        )
        group by teaching_courses.id, subjects.name, student_groups.code,
                 student_groups.education_level, student_groups.study_year
        order by subjects.name, student_groups.code, teaching_courses.id
        limit $3 offset $4
        "#,
        sees_every_course,
        teacher.id().as_uuid(),
        PageRequest::FETCH_LIMIT,
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(TeachingCourseSummary::try_from)
    .collect()
}

#[derive(Debug)]
pub(super) struct StudentGroupRow {
    pub(super) id: Uuid,
    pub(super) code: String,
    pub(super) education_level: String,
    pub(super) study_year: i16,
}

impl TryFrom<StudentGroupRow> for StudentGroup {
    type Error = DbError;

    fn try_from(row: StudentGroupRow) -> Result<Self, Self::Error> {
        let education_level = EducationLevel::parse_str(&row.education_level)
            .ok_or_else(|| DbError::invalid_value(EDUCATION_LEVEL, &row.education_level))?;
        let study_year: u8 = checked_num_cast(STUDY_YEAR, row.study_year)?;

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

#[derive(Debug)]
struct TeachingCourseCardRow {
    id: Uuid,
    subject_id: Uuid,
    student_group_id: Uuid,
    created_at: DateTime<Utc>,
    teacher_ids: Vec<Uuid>,
    subject_name: String,
    group_code: String,
    education_level: String,
    study_year: i16,
}

impl TryFrom<TeachingCourseCardRow> for TeachingCourseSummary {
    type Error = DbError;

    fn try_from(row: TeachingCourseCardRow) -> Result<Self, Self::Error> {
        let subject = Subject::new(SubjectId::from_uuid(row.subject_id), row.subject_name)?;
        let group = StudentGroup::try_from(StudentGroupRow {
            id: row.student_group_id,
            code: row.group_code,
            education_level: row.education_level,
            study_year: row.study_year,
        })?;
        let course = TeachingCourse::try_from(TeachingCourseRow {
            id: row.id,
            subject_id: row.subject_id,
            student_group_id: row.student_group_id,
            created_at: row.created_at,
            teacher_ids: row.teacher_ids,
        })?;

        Ok(Self {
            course,
            subject,
            group,
        })
    }
}
