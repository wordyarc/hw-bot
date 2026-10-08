use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use super::error::ensure_updated;
use super::submissions::{GradeColumns, parse_kind};
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    Assignment, Grade, Subject, Submission, SubmissionId, SubmissionKind, SubmissionStatus,
    TelegramUserId, User,
};
use crate::string_enum::str_enum;

const ATTEMPT_COUNT: &str = "review_notification_outbox.attempt_count";
const OUTCOME: &str = "review_notification_outbox.outcome";
const LOST_LEASE: DbError = DbError::Conflict {
    message: "notification lease is no longer owned by this worker",
};

str_enum! {
    /// Сдача, ожидающая проверки, в outbox не попадает, поэтому исходов только два.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ReviewOutcome {
        Accepted => "accepted",
        Rejected => "rejected",
    }
}

#[derive(Clone, Debug)]
pub struct ClaimedReviewNotification {
    pub id: Uuid,
    pub submission_id: SubmissionId,
    pub recipient_telegram_user_id: TelegramUserId,
    pub subject_name: String,
    pub assignment_title: String,
    pub outcome: ReviewOutcome,
    pub review_comment: String,
    pub kind: SubmissionKind,
    pub grade: Option<Grade>,
    pub attempt_count: u32,
    pub lease_token: Uuid,
}

macro_rules! release_lease {
    ($set:literal, $guard:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query!(
            r#"
            update review_notification_outbox
            set "# + $set + r#",
                lease_token = null,
                lease_expires_at = null
            where id = $1
              and lease_token = $2 "# + $guard + r#"
              and delivered_at is null
              and failed_at is null
            "#
            $(, $arg)*
        )
    };
}

pub async fn enqueue_review_result<'e, E>(
    executor: E,
    submission: &Submission,
    assignment: &Assignment,
    subject: &Subject,
    student: &User,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let outcome = match submission.status() {
        SubmissionStatus::Accepted => ReviewOutcome::Accepted,
        SubmissionStatus::Rejected => ReviewOutcome::Rejected,
        SubmissionStatus::PendingReview => {
            return Err(DbError::Precondition {
                message: "pending submission cannot produce a review notification",
            });
        }
    };
    let review_comment = submission.review_comment().ok_or(DbError::Precondition {
        message: "reviewed submission has no comment",
    })?;

    sqlx::query!(
        r#"
        insert into review_notification_outbox (
            submission_id, recipient_telegram_user_id, subject_name, assignment_title, outcome,
            review_comment
        )
        values ($1, $2, $3, $4, $5, $6)
        "#,
        submission.id().as_uuid(),
        student.telegram_user_id().as_i64(),
        subject.name().as_str(),
        assignment.title().as_str(),
        outcome.as_str(),
        review_comment.as_str(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn claim_next<'e, E>(
    executor: E,
    now: DateTime<Utc>,
    lease_token: Uuid,
    lease_expires_at: DateTime<Utc>,
) -> DbResult<Option<ClaimedReviewNotification>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        ClaimedReviewNotificationRow,
        r#"
        with next_notification as (
            select id
            from review_notification_outbox
            where delivered_at is null
              and failed_at is null
              and available_at <= $1
              and (lease_expires_at is null or lease_expires_at <= $1)
            order by available_at, created_at, id
            for update skip locked
            limit 1
        )
        update review_notification_outbox notification
        set
            lease_token = $2,
            lease_expires_at = $3,
            attempt_count = notification.attempt_count + 1
        from next_notification, submissions
        where notification.id = next_notification.id
          and submissions.id = notification.submission_id
        returning
            notification.id,
            notification.submission_id,
            notification.recipient_telegram_user_id,
            notification.subject_name,
            notification.assignment_title,
            notification.outcome,
            notification.review_comment,
            submissions.kind,
            submissions.raw_score,
            submissions.grade_max_score,
            submissions.late_periods,
            submissions.penalty,
            submissions.penalty_deducted_from,
            submissions.work_score,
            submissions.course_total_penalty,
            notification.attempt_count,
            notification.lease_token as "lease_token!"
        "#,
        now,
        lease_token,
        lease_expires_at,
    )
    .fetch_optional(executor)
    .await?;

    row.map(ClaimedReviewNotification::try_from).transpose()
}

pub async fn mark_delivered<'e, E>(
    executor: E,
    id: Uuid,
    lease_token: Uuid,
    delivered_at: DateTime<Utc>,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let result = release_lease!(
        "delivered_at = $3, last_error = null",
        "",
        id,
        lease_token,
        delivered_at,
    )
    .execute(executor)
    .await?;

    ensure_updated(result, LOST_LEASE)
}

pub async fn reschedule<'e, E>(
    executor: E,
    id: Uuid,
    lease_token: Uuid,
    available_at: DateTime<Utc>,
    last_error: &str,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let result = release_lease!(
        "available_at = $3, last_error = $4",
        "",
        id,
        lease_token,
        available_at,
        last_error,
    )
    .execute(executor)
    .await?;

    ensure_updated(result, LOST_LEASE)
}

pub async fn release_after_credentials_failure<'e, E>(
    executor: E,
    id: Uuid,
    lease_token: Uuid,
    last_error: &str,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let result = release_lease!(
        "attempt_count = attempt_count - 1, last_error = $3",
        "and attempt_count > 0",
        id,
        lease_token,
        last_error,
    )
    .execute(executor)
    .await?;

    ensure_updated(result, LOST_LEASE)
}

pub async fn mark_failed<'e, E>(
    executor: E,
    id: Uuid,
    lease_token: Uuid,
    failed_at: DateTime<Utc>,
    last_error: &str,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let result = release_lease!(
        "failed_at = $3, last_error = $4",
        "",
        id,
        lease_token,
        failed_at,
        last_error,
    )
    .execute(executor)
    .await?;

    ensure_updated(result, LOST_LEASE)
}

#[derive(Debug)]
struct ClaimedReviewNotificationRow {
    id: Uuid,
    submission_id: Uuid,
    recipient_telegram_user_id: i64,
    subject_name: String,
    assignment_title: String,
    outcome: String,
    review_comment: String,
    kind: String,
    raw_score: Option<i16>,
    grade_max_score: Option<i16>,
    late_periods: Option<i32>,
    penalty: Option<i16>,
    penalty_deducted_from: Option<String>,
    work_score: Option<i16>,
    course_total_penalty: Option<i16>,
    attempt_count: i32,
    lease_token: Uuid,
}

impl TryFrom<ClaimedReviewNotificationRow> for ClaimedReviewNotification {
    type Error = DbError;

    fn try_from(row: ClaimedReviewNotificationRow) -> Result<Self, Self::Error> {
        let outcome = ReviewOutcome::parse_str(&row.outcome)
            .ok_or_else(|| DbError::invalid_value(OUTCOME, &row.outcome))?;

        Ok(Self {
            id: row.id,
            submission_id: SubmissionId::from_uuid(row.submission_id),
            recipient_telegram_user_id: TelegramUserId::new(row.recipient_telegram_user_id)?,
            subject_name: row.subject_name,
            assignment_title: row.assignment_title,
            outcome,
            review_comment: row.review_comment,
            kind: parse_kind(&row.kind)?,
            grade: Grade::restore(
                GradeColumns {
                    raw_score: row.raw_score,
                    max_score: row.grade_max_score,
                    late_periods: row.late_periods,
                    penalty: row.penalty,
                    penalty_deducted_from: row.penalty_deducted_from,
                    work_score: row.work_score,
                    course_total_penalty: row.course_total_penalty,
                }
                .parts()?,
            )?,
            attempt_count: checked_num_cast(ATTEMPT_COUNT, row.attempt_count)?,
            lease_token: row.lease_token,
        })
    }
}
