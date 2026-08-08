use sqlx::{Postgres, Transaction};

use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{AssignmentId, StudentAssignmentProgress, UserId};

const EXTRA_ATTEMPTS: &str = "student_assignment_progress.extra_attempts";

pub async fn lock_or_create(
    tx: &mut Transaction<'_, Postgres>,
    assignment_id: AssignmentId,
    student_id: UserId,
) -> DbResult<u16> {
    let extra_attempts = sqlx::query_scalar!(
        r#"
        insert into student_assignment_progress (assignment_id, student_id, extra_attempts)
        values ($1, $2, 0)
        on conflict (assignment_id, student_id) do update
        set extra_attempts = student_assignment_progress.extra_attempts
        returning extra_attempts
        "#,
        assignment_id.as_uuid(),
        student_id.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await?;

    checked_num_cast(EXTRA_ATTEMPTS, extra_attempts)
}

pub async fn update(
    tx: &mut Transaction<'_, Postgres>,
    progress: &StudentAssignmentProgress,
) -> DbResult<()> {
    let result = sqlx::query!(
        r#"
        update student_assignment_progress
        set extra_attempts = $3
        where assignment_id = $1
          and student_id = $2
        "#,
        progress.assignment_id().as_uuid(),
        progress.student_id().as_uuid(),
        checked_num_cast::<_, i16>(EXTRA_ATTEMPTS, progress.extra_attempts())?,
    )
    .execute(&mut **tx)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::Conflict {
            message: "student assignment progress was removed by a concurrent request",
        });
    }

    Ok(())
}
