use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;

use crate::db::academic::StudentGroupRow;
use crate::db::ensure_updated;
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{InviteGeneration, StudentGroup, StudentGroupId};

const GENERATION: &str = "student_group_invites.generation";

pub async fn ensure<'e, E>(executor: E, group_id: StudentGroupId) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into student_group_invites (student_group_id, generation)
        values ($1, 1)
        on conflict (student_group_id) do nothing
        "#,
        group_id.as_uuid(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn find_generation<'e, E>(
    executor: E,
    group_id: StudentGroupId,
) -> DbResult<Option<InviteGeneration>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_scalar!(
        r#"
        select generation
        from student_group_invites
        where student_group_id = $1
        "#,
        group_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?
    .map(restore_generation)
    .transpose()
}

pub async fn rotate<'e, E>(
    executor: E,
    group_id: StudentGroupId,
    expected: InviteGeneration,
) -> DbResult<InviteGeneration>
where
    E: PgExecutor<'e>,
{
    let rotated = expected.rotated()?;
    let result = sqlx::query!(
        r#"
        update student_group_invites
        set generation = $3, updated_at = now()
        where student_group_id = $1 and generation = $2
        "#,
        group_id.as_uuid(),
        i64::from(expected.as_u32()),
        i64::from(rotated.as_u32()),
    )
    .execute(executor)
    .await?;

    ensure_updated(
        result,
        DbError::Conflict {
            message: "group invite was rotated by a concurrent request",
        },
    )?;

    Ok(rotated)
}

pub async fn lock_group_for_registration(
    conn: &mut PgConnection,
    group_id: StudentGroupId,
    generation: InviteGeneration,
) -> DbResult<Option<StudentGroup>> {
    let row = sqlx::query_as!(
        StudentGroupRow,
        r#"
        select
            student_groups.id,
            student_groups.code,
            student_groups.education_level,
            student_groups.study_year
        from student_group_invites
        join student_groups on student_groups.id = student_group_invites.student_group_id
        where student_group_invites.student_group_id = $1
          and student_group_invites.generation = $2
        for share of student_group_invites
        "#,
        group_id.as_uuid(),
        i64::from(generation.as_u32()),
    )
    .fetch_optional(&mut *conn)
    .await?;

    row.map(StudentGroup::try_from).transpose()
}

fn restore_generation(value: i64) -> DbResult<InviteGeneration> {
    Ok(InviteGeneration::new(checked_num_cast(GENERATION, value)?)?)
}
