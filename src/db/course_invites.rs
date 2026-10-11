use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;

use crate::db::academic::find_teaching_course_summary;
use crate::db::ensure_updated;
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{InviteGeneration, TeachingCourseId};
use crate::read_models::TeachingCourseSummary;

const GENERATION: &str = "teaching_course_invites.generation";

pub async fn ensure<'e, E>(executor: E, course_id: TeachingCourseId) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into teaching_course_invites (teaching_course_id, generation)
        values ($1, 1)
        on conflict (teaching_course_id) do nothing
        "#,
        course_id.as_uuid(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn find_generation<'e, E>(
    executor: E,
    course_id: TeachingCourseId,
) -> DbResult<Option<InviteGeneration>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_scalar!(
        r#"
        select generation
        from teaching_course_invites
        where teaching_course_id = $1
        "#,
        course_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?
    .map(restore_generation)
    .transpose()
}

pub async fn rotate<'e, E>(
    executor: E,
    course_id: TeachingCourseId,
    expected: InviteGeneration,
) -> DbResult<InviteGeneration>
where
    E: PgExecutor<'e>,
{
    let rotated = expected.rotated()?;
    let result = sqlx::query!(
        r#"
        update teaching_course_invites
        set generation = $3, updated_at = now()
        where teaching_course_id = $1 and generation = $2
        "#,
        course_id.as_uuid(),
        i64::from(expected.as_u32()),
        i64::from(rotated.as_u32()),
    )
    .execute(executor)
    .await?;

    ensure_updated(
        result,
        DbError::Conflict {
            message: "course invite was rotated by a concurrent request",
        },
    )?;

    Ok(rotated)
}

pub async fn lock_course_for_registration(
    conn: &mut PgConnection,
    course_id: TeachingCourseId,
    generation: InviteGeneration,
) -> DbResult<Option<TeachingCourseSummary>> {
    let locked = sqlx::query_scalar!(
        r#"
        select teaching_course_id
        from teaching_course_invites
        where teaching_course_id = $1
          and generation = $2
        for share
        "#,
        course_id.as_uuid(),
        i64::from(generation.as_u32()),
    )
    .fetch_optional(&mut *conn)
    .await?;

    match locked {
        Some(_) => find_teaching_course_summary(conn, course_id).await,
        None => Ok(None),
    }
}

fn restore_generation(value: i64) -> DbResult<InviteGeneration> {
    Ok(InviteGeneration::new(checked_num_cast(GENERATION, value)?)?)
}
