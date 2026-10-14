use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use teloxide::prelude::*;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::network::TimedRequest;
use super::text::{MESSAGE_LIMIT, truncate};
use super::{DeliveryFailure, delivery_failure, request_error_kind};
use crate::bot::texts;
use crate::db::notifications::ReviewOutcome;
use crate::db::{self, DbError};
use crate::domain::{Grade, SubmissionKind};
use crate::health::ComponentHealth;

#[derive(Clone, Debug)]
pub struct NotificationWorkerSettings {
    pub request_timeout: Duration,
    pub poll_interval: Duration,
    pub lease_duration: Duration,
    pub max_attempts: u32,
    pub retry_initial: Duration,
    pub retry_max: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryOutcome {
    Idle,
    Delivered,
    Rescheduled,
    Failed,
    RateLimited { retry_after: Duration },
}

#[derive(Debug, Error)]
pub enum NotificationWorkerError {
    #[error("Telegram credentials were rejected")]
    InvalidCredentials,

    #[error("Telegram notification worker startup failed: {0}")]
    Startup(&'static str),

    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
pub struct NotificationWorker {
    pool: PgPool,
    telegram: Bot,
    settings: NotificationWorkerSettings,
}

impl NotificationWorker {
    pub fn new(pool: PgPool, telegram: Bot, settings: NotificationWorkerSettings) -> Self {
        Self {
            pool,
            telegram,
            settings,
        }
    }

    pub async fn deliver_one(
        &self,
        now: DateTime<Utc>,
    ) -> Result<DeliveryOutcome, NotificationWorkerError> {
        let lease_token = Uuid::new_v4();
        let lease_expires_at = after(now, self.settings.lease_duration);
        let Some(notification) =
            db::notifications::claim_next(&self.pool, now, lease_token, lease_expires_at).await?
        else {
            return Ok(DeliveryOutcome::Idle);
        };

        let text = render_review_notification(
            notification.outcome,
            notification.kind,
            &notification.subject_name,
            &notification.assignment_title,
            notification.grade,
            &notification.review_comment,
        );

        let result = TimedRequest::new(
            self.telegram.send_message(
                ChatId(notification.recipient_telegram_user_id.as_i64()),
                text,
            ),
            self.settings.request_timeout,
        )
        .await;
        let Err(error) = result else {
            db::notifications::mark_delivered(
                &self.pool,
                notification.id,
                notification.lease_token,
                Utc::now(),
            )
            .await?;
            return Ok(DeliveryOutcome::Delivered);
        };
        let failure = delivery_failure(&error);
        // Из InvalidJson и Unknown берём только категорию: внутри возможны токен и текст пользователя.
        let last_error = request_error_kind(&error);
        if failure == DeliveryFailure::InvalidCredentials {
            db::notifications::release_after_credentials_failure(
                &self.pool,
                notification.id,
                notification.lease_token,
                last_error,
            )
            .await?;
            return Err(NotificationWorkerError::InvalidCredentials);
        }

        let retry_after = match failure {
            DeliveryFailure::RateLimited(Some(delay)) => delay,
            _ => retry_delay(&self.settings, notification.attempt_count),
        };
        let rate_limited = matches!(failure, DeliveryFailure::RateLimited(_));
        if rate_limited {
            tracing::warn!(
                retry_after_seconds = retry_after.as_secs(),
                "Telegram rate limited a notification request"
            );
        }
        let outcome = if failure == DeliveryFailure::Permanent
            || notification.attempt_count >= self.settings.max_attempts
        {
            db::notifications::mark_failed(
                &self.pool,
                notification.id,
                notification.lease_token,
                Utc::now(),
                last_error,
            )
            .await?;
            tracing::warn!(
                notification_id = %notification.id,
                submission_id = %notification.submission_id.as_uuid(),
                attempt = notification.attempt_count,
                kind = last_error,
                reason = if failure == DeliveryFailure::Permanent { "permanent_rejection" } else { "attempts_exhausted" },
                "notification delivery failed permanently"
            );
            DeliveryOutcome::Failed
        } else {
            let available_at = after(Utc::now(), retry_after);
            db::notifications::reschedule(
                &self.pool,
                notification.id,
                notification.lease_token,
                available_at,
                last_error,
            )
            .await?;
            tracing::debug!(notification_id = %notification.id, attempt = notification.attempt_count, kind = last_error, "notification rescheduled");
            DeliveryOutcome::Rescheduled
        };
        Ok(if rate_limited {
            DeliveryOutcome::RateLimited { retry_after }
        } else {
            outcome
        })
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        health: ComponentHealth,
    ) -> Result<(), NotificationWorkerError> {
        let Some(result) = cancellation
            .run_until_cancelled(
                TimedRequest::new(self.telegram.get_me(), self.settings.request_timeout).send(),
            )
            .await
        else {
            return Ok(());
        };
        match result {
            Ok(_) => {}
            Err(error) if delivery_failure(&error) == DeliveryFailure::InvalidCredentials => {
                return Err(NotificationWorkerError::InvalidCredentials);
            }
            Err(error) => return Err(NotificationWorkerError::Startup(request_error_kind(&error))),
        }
        let _ready = health.ready();
        let mut database_failures = DatabaseFailures::default();

        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }

            let delivery = self.deliver_one(Utc::now()).await;
            if delivery.is_ok() {
                database_failures.recover();
            }
            match delivery {
                Ok(DeliveryOutcome::RateLimited { retry_after }) => {
                    cancellation
                        .run_until_cancelled(tokio::time::sleep(retry_after))
                        .await;
                }
                Ok(DeliveryOutcome::Idle) => {
                    cancellation
                        .run_until_cancelled(tokio::time::sleep(self.settings.poll_interval))
                        .await;
                }
                Ok(_) => {}
                Err(NotificationWorkerError::InvalidCredentials) => {
                    return Err(NotificationWorkerError::InvalidCredentials);
                }
                Err(error) => {
                    database_failures.record(&error);
                    cancellation
                        .run_until_cancelled(tokio::time::sleep(self.settings.poll_interval))
                        .await;
                }
            }
        }
    }
}

#[derive(Default)]
struct DatabaseFailures(u64);

impl DatabaseFailures {
    fn record(&mut self, error: &NotificationWorkerError) {
        self.0 += 1;
        if self.0.is_power_of_two() {
            tracing::error!(
                error = ?error,
                failures = self.0,
                "notification worker failed to access database"
            );
        }
    }

    fn recover(&mut self) {
        if self.0 > 0 {
            tracing::info!(
                failures = self.0,
                "notification worker regained database access"
            );
            self.0 = 0;
        }
    }
}

fn render_review_notification(
    outcome: ReviewOutcome,
    kind: SubmissionKind,
    subject_name: &str,
    assignment_title: &str,
    grade: Option<Grade>,
    review_comment: &str,
) -> String {
    let verdict = match outcome {
        ReviewOutcome::Accepted => texts::REVIEW_ACCEPTED,
        ReviewOutcome::Rejected => texts::REVIEW_REJECTED,
    };
    let text = texts::review_notification(
        verdict,
        subject_name,
        assignment_title,
        kind,
        grade,
        review_comment,
    );

    truncate(&text, MESSAGE_LIMIT)
}

fn retry_delay(settings: &NotificationWorkerSettings, attempt_count: u32) -> Duration {
    let multiplier = 1_u32 << attempt_count.saturating_sub(1).min(31);

    settings
        .retry_initial
        .checked_mul(multiplier)
        .unwrap_or(settings.retry_max)
        .min(settings.retry_max)
}

/// Непредставимая задержка означает «никогда»: запись просто не станет доступной раньше срока.
fn after(now: DateTime<Utc>, delay: Duration) -> DateTime<Utc> {
    chrono::Duration::from_std(delay)
        .ok()
        .and_then(|delay| now.checked_add_signed(delay))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DeductedFrom, LatePenalty, LatePolicy};
    use crate::test_support::logs::LogBuffer;

    #[test]
    fn review_message_is_bounded_by_telegram_limit() {
        let message = render_review_notification(
            ReviewOutcome::Rejected,
            SubmissionKind::Regular,
            "Backend development",
            "Ownership",
            None,
            &"🦀".repeat(MESSAGE_LIMIT),
        );

        assert!(message.encode_utf16().count() <= MESSAGE_LIMIT);
        assert!(message.ends_with("..."));
    }

    #[test]
    fn review_message_names_the_subject_the_assignment_belongs_to() {
        let message = render_review_notification(
            ReviewOutcome::Accepted,
            SubmissionKind::Regular,
            "Backend development",
            "Ownership",
            None,
            "95/100",
        );

        assert!(message.contains("Backend development"), "{message}");
        assert!(message.contains("Ownership"), "{message}");
    }

    fn accepted(grade: Option<Grade>) -> String {
        accepted_as(SubmissionKind::Regular, grade)
    }

    fn accepted_as(kind: SubmissionKind, grade: Option<Grade>) -> String {
        render_review_notification(
            ReviewOutcome::Accepted,
            kind,
            "Backend development",
            "Ownership",
            grade,
            "Хорошо",
        )
    }

    fn grade_line(grade: Grade) -> String {
        texts::grade_line(SubmissionKind::Regular, Some(grade)).unwrap()
    }

    fn late_by_one_period(deducted_from: DeductedFrom) -> Option<LatePenalty> {
        LatePolicy::new(7, 4, 4, deducted_from)
            .unwrap()
            .late_penalty(1)
    }

    #[test]
    fn an_accepted_scored_review_names_the_grade_on_time_or_late() {
        for grade in [
            Grade::new(4, 5, None),
            Grade::new(8, 10, late_by_one_period(DeductedFrom::WorkScore)),
            Grade::new(8, 10, late_by_one_period(DeductedFrom::CourseTotal)),
        ] {
            let message = accepted(Some(grade));

            assert!(message.contains(&grade_line(grade)), "{message}");
        }
    }

    #[test]
    fn a_credited_makeup_is_named_with_its_credit_and_without_a_penalty() {
        let grade = Grade::new(6, 10, None);
        let message = accepted_as(SubmissionKind::Makeup, Some(grade));

        assert!(
            message.contains(&texts::grade_line(SubmissionKind::Makeup, Some(grade)).unwrap()),
            "{message}"
        );
        assert!(message.contains(texts::MAKEUP_MARK), "{message}");
        assert!(!message.contains('−'), "{message}");
    }

    #[test]
    fn a_pass_fail_review_names_no_score() {
        let grade = Grade::new(4, 5, None);
        let line = format!("\n{}", grade_line(grade));

        assert_eq!(accepted(Some(grade)).replace(&line, ""), accepted(None));
    }

    #[test]
    fn an_unrepresentable_delay_means_never_instead_of_an_error() {
        let now = Utc::now();

        assert_eq!(
            after(now, Duration::from_secs(60)),
            now + chrono::Duration::seconds(60)
        );
        assert_eq!(after(now, Duration::MAX), DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn repeated_database_failures_are_logged_sparsely_until_recovery() {
        let logs = LogBuffer::of(|| {
            let error = NotificationWorkerError::Database(DbError::Conflict {
                message: "unavailable",
            });
            let mut failures = DatabaseFailures::default();
            failures.recover();
            for _ in 0..5 {
                failures.record(&error);
            }
            failures.recover();
            failures.recover();
            failures.record(&error);
        });
        let logged: Vec<_> = logs
            .events()
            .into_iter()
            .map(|event| (event["level"].clone(), event["fields"]["failures"].clone()))
            .collect();

        assert_eq!(
            logged,
            [
                ("ERROR".into(), 1.into()),
                ("ERROR".into(), 2.into()),
                ("ERROR".into(), 4.into()),
                ("INFO".into(), 5.into()),
                ("ERROR".into(), 1.into()),
            ]
        );
    }

    #[test]
    fn retry_delay_grows_and_is_capped() {
        let worker = NotificationWorkerSettings {
            request_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_secs(1),
            lease_duration: Duration::from_secs(10),
            max_attempts: 10,
            retry_initial: Duration::from_secs(2),
            retry_max: Duration::from_secs(5),
        };

        assert_eq!(retry_delay(&worker, 1), Duration::from_secs(2));
        assert_eq!(retry_delay(&worker, 2), Duration::from_secs(4));
        assert_eq!(retry_delay(&worker, 3), Duration::from_secs(5));
    }
}
