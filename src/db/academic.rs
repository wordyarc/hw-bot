use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, checked_num_cast, course_invites};
use crate::domain::{
    EducationLevel, StudentGroup, StudentGroupId, StudyProgram, Subject, SubjectId, TeachingCourse,
    TeachingCourseId, User, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::TeachingCourseSummary;

const EDUCATION_LEVEL: &str = "student_groups.education_level";
const STUDY_YEAR: &str = "student_groups.study_year";

macro_rules! select_course_summaries {
    ($tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            TeachingCourseSummaryRow,
            r#"
            select
                teaching_courses.id,
                teaching_courses.subject_id,
                teaching_courses.created_at,
                array(
                    select teacher_id
                    from teaching_course_teachers
                    where teaching_course_id = teaching_courses.id
                ) as "teacher_ids!: Vec<Uuid>",
                subjects.name as subject_name,
                course_groups.ids as "group_ids!: Vec<Uuid>",
                course_groups.codes as "group_codes!: Vec<String>",
                course_groups.education_levels as "group_education_levels!: Vec<String>",
                course_groups.study_years as "group_study_years!: Vec<i16>"
            from teaching_courses
            join subjects
              on subjects.id = teaching_courses.subject_id
            cross join lateral (
                select
                    coalesce(array_agg(student_groups.id order by student_groups.code), '{}')
                        as ids,
                    coalesce(array_agg(student_groups.code order by student_groups.code), '{}')
                        as codes,
                    coalesce(
                        array_agg(student_groups.education_level order by student_groups.code),
                        '{}'
                    ) as education_levels,
                    coalesce(
                        array_agg(student_groups.study_year order by student_groups.code),
                        '{}'
                    ) as study_years,
                    min(student_groups.code) as first_code
                from teaching_course_groups
                join student_groups
                  on student_groups.id = teaching_course_groups.student_group_id
                where teaching_course_groups.teaching_course_id = teaching_courses.id
            ) course_groups
            "# + $tail
            $(, $arg)*
        )
    };
}

pub async fn insert_student_group(
    executor: impl PgExecutor<'_>,
    group: &StudentGroup,
) -> DbResult<()> {
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

pub async fn find_student_group_by_id(
    executor: impl PgExecutor<'_>,
    id: StudentGroupId,
) -> DbResult<Option<StudentGroup>> {
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

pub async fn insert_subject(executor: impl PgExecutor<'_>, subject: &Subject) -> DbResult<()> {
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

pub async fn find_subject_with_sandbox(
    executor: impl PgExecutor<'_>,
    id: SubjectId,
) -> DbResult<Option<(Subject, bool)>> {
    let row = sqlx::query!(
        r#"
        select id, name, sandbox
        from subjects
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(|row| {
        let subject = Subject::new(SubjectId::from_uuid(row.id), row.name)?;
        Ok((subject, row.sandbox))
    })
    .transpose()
}

/// Вызывающий обязан объединить все INSERT транзакцией: нельзя оставить курс без групп и преподавателей.
pub async fn insert_teaching_course(
    conn: &mut PgConnection,
    course: &TeachingCourse,
) -> DbResult<()> {
    sqlx::query!(
        r#"
        insert into teaching_courses (id, subject_id, created_at)
        values ($1, $2, $3)
        "#,
        course.id().as_uuid(),
        course.subject_id().as_uuid(),
        course.created_at(),
    )
    .execute(&mut *conn)
    .await?;

    let group_ids: Vec<Uuid> = course.group_ids().iter().map(|id| id.as_uuid()).collect();

    sqlx::query!(
        r#"
        insert into teaching_course_groups (teaching_course_id, subject_id, student_group_id)
        select $1, $2, student_group_id
        from unnest($3::uuid[]) as course_groups (student_group_id)
        "#,
        course.id().as_uuid(),
        course.subject_id().as_uuid(),
        &group_ids,
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
    .execute(&mut *conn)
    .await?;

    course_invites::ensure(conn, course.id()).await
}

pub async fn add_teaching_course_group(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
) -> DbResult<()> {
    sqlx::query!(
        r#"
        insert into teaching_course_groups (teaching_course_id, subject_id, student_group_id)
        select id, subject_id, $2
        from teaching_courses
        where id = $1
        on conflict (teaching_course_id, student_group_id) do nothing
        "#,
        course_id.as_uuid(),
        group_id.as_uuid(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn find_teaching_course_by_id(
    executor: impl PgExecutor<'_>,
    id: TeachingCourseId,
) -> DbResult<Option<TeachingCourse>> {
    let row = sqlx::query_as!(
        TeachingCourseRow,
        r#"
        select
            id,
            subject_id,
            created_at,
            array(
                select student_group_id
                from teaching_course_groups
                where teaching_course_id = teaching_courses.id
            ) as "group_ids!: Vec<Uuid>",
            array(
                select teacher_id
                from teaching_course_teachers
                where teaching_course_id = teaching_courses.id
            ) as "teacher_ids!: Vec<Uuid>"
        from teaching_courses
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(TeachingCourse::try_from).transpose()
}

pub(crate) async fn find_teaching_course_summary(
    executor: impl PgExecutor<'_>,
    id: TeachingCourseId,
) -> DbResult<Option<TeachingCourseSummary>> {
    select_course_summaries!(" where teaching_courses.id = $1", id.as_uuid())
        .fetch_optional(executor)
        .await?
        .map(TeachingCourseSummary::try_from)
        .transpose()
}

pub(crate) async fn list_teaching_courses_for_teacher(
    executor: impl PgExecutor<'_>,
    teacher: &User,
    page: PageRequest,
) -> DbResult<Vec<TeachingCourseSummary>> {
    let sees_every_course = teacher.role().can_manage_all_teaching_courses();

    select_course_summaries!(
        r#"
        where $1 or exists (
            select 1
            from teaching_course_teachers mine
            where mine.teaching_course_id = teaching_courses.id
              and mine.teacher_id = $2
        )
        order by subjects.name, course_groups.first_code, teaching_courses.id
        limit $3 offset $4
        "#,
        sees_every_course,
        teacher.id().as_uuid(),
        page.fetch_limit(),
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
    created_at: DateTime<Utc>,
    group_ids: Vec<Uuid>,
    teacher_ids: Vec<Uuid>,
}

impl TryFrom<TeachingCourseRow> for TeachingCourse {
    type Error = DbError;

    fn try_from(row: TeachingCourseRow) -> Result<Self, Self::Error> {
        let group_ids: BTreeSet<StudentGroupId> = row
            .group_ids
            .into_iter()
            .map(StudentGroupId::from_uuid)
            .collect();
        let teacher_ids: HashSet<UserId> =
            row.teacher_ids.into_iter().map(UserId::from_uuid).collect();

        Ok(TeachingCourse::restore(
            TeachingCourseId::from_uuid(row.id),
            SubjectId::from_uuid(row.subject_id),
            group_ids,
            teacher_ids,
            row.created_at,
        )?)
    }
}

#[derive(Debug)]
struct TeachingCourseSummaryRow {
    id: Uuid,
    subject_id: Uuid,
    created_at: DateTime<Utc>,
    teacher_ids: Vec<Uuid>,
    subject_name: String,
    group_ids: Vec<Uuid>,
    group_codes: Vec<String>,
    group_education_levels: Vec<String>,
    group_study_years: Vec<i16>,
}

impl TryFrom<TeachingCourseSummaryRow> for TeachingCourseSummary {
    type Error = DbError;

    fn try_from(row: TeachingCourseSummaryRow) -> Result<Self, Self::Error> {
        let subject = Subject::new(SubjectId::from_uuid(row.subject_id), row.subject_name)?;
        let groups = row
            .group_ids
            .iter()
            .zip(row.group_codes)
            .zip(row.group_education_levels)
            .zip(row.group_study_years)
            .map(|(((&id, code), education_level), study_year)| {
                StudentGroup::try_from(StudentGroupRow {
                    id,
                    code,
                    education_level,
                    study_year,
                })
            })
            .collect::<DbResult<Vec<_>>>()?;
        let course = TeachingCourse::try_from(TeachingCourseRow {
            id: row.id,
            subject_id: row.subject_id,
            created_at: row.created_at,
            group_ids: row.group_ids,
            teacher_ids: row.teacher_ids,
        })?;

        Ok(Self {
            course,
            subject,
            groups,
        })
    }
}
