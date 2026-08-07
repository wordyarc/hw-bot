use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use crate::db::{DbError, DbResult, to_db_smallint, try_u16_from_i16, try_u16_from_i64};
use crate::domain::{
    AssignmentId, DomainError, Role, Submission, SubmissionHistory, SubmissionId, SubmissionStatus,
    User, UserId,
};

/// Postgres class 23505: `unique_violation`.
const UNIQUE_VIOLATION: &str = "23505";

macro_rules! columns {
    () => {
        "id, assignment_id, student_id, attempt_number, content, status, submitted_at, \
         reviewed_by, reviewed_at, review_comment"
    };
}

#[derive(Clone, Debug, Default)]
pub struct SubmissionRepository;

impl SubmissionRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn find_by_id<'e, E>(
        &self,
        executor: E,
        id: SubmissionId,
    ) -> DbResult<Option<Submission>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, SubmissionRow>(concat!(
            "select ",
            columns!(),
            " from submissions where id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(executor)
        .await?;

        row.map(Submission::try_from).transpose()
    }

    pub async fn insert<'e, E>(&self, executor: E, submission: &Submission) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query(concat!(
            "insert into submissions (",
            columns!(),
            ") values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
        ))
        .bind(submission.id().as_uuid())
        .bind(submission.assignment_id().as_uuid())
        .bind(submission.student_id().as_uuid())
        .bind(to_db_smallint(
            "submission.attempt_number",
            submission.attempt_number(),
        )?)
        .bind(submission.content().as_str())
        .bind(submission.status().as_db_str())
        .bind(submission.submitted_at())
        .bind(submission.reviewed_by().map(UserId::as_uuid))
        .bind(submission.reviewed_at())
        .bind(submission.review_comment().map(|comment| comment.as_str()))
        .execute(executor)
        .await
        .map_err(insert_conflict_or)?;

        Ok(())
    }

    pub async fn save_review_result<'e, E>(
        &self,
        executor: E,
        submission: &Submission,
    ) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        if submission.status() == SubmissionStatus::PendingReview {
            return Err(DbError::Conflict {
                message: "pending submission cannot be saved as a review result",
            });
        }

        let result = sqlx::query(
            r#"
            update submissions
            set
                status = $2,
                reviewed_by = $3,
                reviewed_at = $4,
                review_comment = $5
            where id = $1
              and status = 'pending_review'
            "#,
        )
        .bind(submission.id().as_uuid())
        .bind(submission.status().as_db_str())
        .bind(submission.reviewed_by().map(UserId::as_uuid))
        .bind(submission.reviewed_at())
        .bind(submission.review_comment().map(|comment| comment.as_str()))
        .execute(executor)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::RuleViolation(
                DomainError::SubmissionIsNotPendingReview,
            ));
        }

        Ok(())
    }

    pub async fn load_history<'e, E>(
        &self,
        executor: E,
        assignment_id: AssignmentId,
        student_id: UserId,
    ) -> DbResult<SubmissionHistory>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, SubmissionHistoryRow>(
            r#"
            with history as (
                select
                    count(*) filter (
                        where status in ('accepted', 'rejected')
                    )::bigint as used_attempts,
                    coalesce(bool_or(status = 'accepted'), false) as has_accepted_submission,
                    coalesce(bool_or(status = 'pending_review'), false) as has_pending_submission
                from submissions
                where assignment_id = $1
                  and student_id = $2
            ),
            progress as (
                select coalesce(max(extra_attempts), 0)::smallint as extra_attempts
                from student_assignment_progress
                where assignment_id = $1
                  and student_id = $2
            )
            select
                history.used_attempts,
                progress.extra_attempts,
                history.has_accepted_submission,
                history.has_pending_submission
            from history
            cross join progress
            "#,
        )
        .bind(assignment_id.as_uuid())
        .bind(student_id.as_uuid())
        .fetch_one(executor)
        .await?;

        Ok(SubmissionHistory::new(
            try_u16_from_i64("submission_history.used_attempts", row.used_attempts)?,
            try_u16_from_i16("submission_history.extra_attempts", row.extra_attempts)?,
            row.has_accepted_submission,
            row.has_pending_submission,
        ))
    }

    pub async fn lock_next_pending_for_reviewer(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        reviewer: &User,
    ) -> DbResult<Option<Submission>> {
        let can_review_all = matches!(reviewer.role(), Role::Admin);
        let locked_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            select submissions.id
            from submissions
            join assignments
              on assignments.id = submissions.assignment_id
            left join teaching_course_teachers
              on teaching_course_teachers.teaching_course_id = assignments.teaching_course_id
             and teaching_course_teachers.teacher_id = $2
            where submissions.status = 'pending_review'
              and ($1 or teaching_course_teachers.teacher_id is not null)
            order by submissions.submitted_at, submissions.id
            for update of submissions skip locked
            limit 1
            "#,
        )
        .bind(can_review_all)
        .bind(reviewer.id().as_uuid())
        .fetch_optional(&mut **tx)
        .await?;

        let Some(locked_id) = locked_id else {
            return Ok(None);
        };

        self.find_by_id(&mut **tx, SubmissionId::from_uuid(locked_id))
            .await
    }
}

#[derive(Debug, FromRow)]
struct SubmissionRow {
    id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
    attempt_number: i16,
    content: String,
    status: String,
    submitted_at: DateTime<Utc>,
    reviewed_by: Option<Uuid>,
    reviewed_at: Option<DateTime<Utc>>,
    review_comment: Option<String>,
}

impl TryFrom<SubmissionRow> for Submission {
    type Error = DbError;

    fn try_from(row: SubmissionRow) -> Result<Self, Self::Error> {
        let status = SubmissionStatus::from_db_str(&row.status)?;
        let attempt_number = try_u16_from_i16("submissions.attempt_number", row.attempt_number)?;

        Ok(Submission::restore(
            SubmissionId::from_uuid(row.id),
            AssignmentId::from_uuid(row.assignment_id),
            UserId::from_uuid(row.student_id),
            attempt_number,
            row.content,
            status,
            row.submitted_at,
            row.reviewed_by.map(UserId::from_uuid),
            row.reviewed_at,
            row.review_comment,
        )?)
    }
}

#[derive(Debug, FromRow)]
struct SubmissionHistoryRow {
    used_attempts: i64,
    extra_attempts: i16,
    has_accepted_submission: bool,
    has_pending_submission: bool,
}

fn insert_conflict_or(error: sqlx::Error) -> DbError {
    let Some(database_error) = error.as_database_error() else {
        return DbError::Database(error);
    };

    if database_error.code().as_deref() != Some(UNIQUE_VIOLATION) {
        return DbError::Database(error);
    }

    match database_error.constraint() {
        Some("submissions_one_pending_per_student_assignment_idx") => {
            DbError::RuleViolation(DomainError::SubmissionAlreadyPendingReview)
        }
        Some("submissions_one_accepted_per_student_assignment_idx") => {
            DbError::RuleViolation(DomainError::SubmissionAlreadyAccepted)
        }
        Some("submissions_assignment_id_student_id_attempt_number_key") => DbError::Conflict {
            message: "attempt number was already taken by a concurrent submission",
        },
        _ => DbError::Database(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_non_unique_violations_as_database_errors() {
        assert!(matches!(
            insert_conflict_or(sqlx::Error::RowNotFound),
            DbError::Database(_)
        ));
    }
}
