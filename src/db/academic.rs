use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::DbResult;
use crate::domain::{StudentGroup, Subject, TeachingCourse};

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
