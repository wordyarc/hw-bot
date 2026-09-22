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
use crate::db::{self, DbError, DbResult};
use crate::domain::SubmissionStatus;
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
        let lease_expires_at = now + chrono_duration(self.settings.lease_duration)?;
        let Some(notification) =
            db::notifications::claim_next(&self.pool, now, lease_token, lease_expires_at).await?
        else {
            return Ok(DeliveryOutcome::Idle);
        };

        let text = render_review_notification(
            notification.outcome,
            &notification.subject_name,
            &notification.assignment_title,
            notification.final_score.zip(notification.max_score),
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
            let available_at = Utc::now() + chrono_duration(retry_after)?;
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

        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }

            match self.deliver_one(Utc::now()).await {
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
                    tracing::error!(error = ?error, "notification worker failed to access database");
                    cancellation
                        .run_until_cancelled(tokio::time::sleep(self.settings.poll_interval))
                        .await;
                }
            }
        }
    }
}

fn render_review_notification(
    outcome: SubmissionStatus,
    subject_name: &str,
    assignment_title: &str,
    score: Option<(u16, u16)>,
    review_comment: &str,
) -> String {
    let verdict = match outcome {
        SubmissionStatus::Accepted => "✅ Работа принята.",
        SubmissionStatus::Rejected => "❌ Работа отклонена.",
        SubmissionStatus::PendingReview => unreachable!("outbox contains only completed reviews"),
    };
    let score = score
        .map(|(final_score, max_score)| format!("\n🏅 Балл: {final_score} из {max_score}"))
        .unwrap_or_default();
    let text = format!(
        "{verdict}\n📘 Предмет: {subject_name}\n📚 Задание: {assignment_title}{score}\n\n\
         💬 Комментарий преподавателя:\n{review_comment}"
    );

    if text.chars().count() <= MESSAGE_LIMIT {
        text
    } else {
        truncate(&text, MESSAGE_LIMIT - 3, true)
    }
}

fn retry_delay(settings: &NotificationWorkerSettings, attempt_count: u32) -> Duration {
    let multiplier = 1_u32 << attempt_count.saturating_sub(1).min(31);

    settings
        .retry_initial
        .checked_mul(multiplier)
        .unwrap_or(settings.retry_max)
        .min(settings.retry_max)
}

fn chrono_duration(duration: Duration) -> DbResult<chrono::Duration> {
    chrono::Duration::from_std(duration).map_err(|_| DbError::Conflict {
        message: "notification worker duration is too large",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_message_is_bounded_by_telegram_limit() {
        let message = render_review_notification(
            SubmissionStatus::Rejected,
            "Backend development",
            "Ownership",
            None,
            &"x".repeat(MESSAGE_LIMIT * 2),
        );

        assert_eq!(message.chars().count(), MESSAGE_LIMIT);
        assert!(message.ends_with("..."));
    }

    #[test]
    fn review_message_names_the_subject_the_assignment_belongs_to() {
        let message = render_review_notification(
            SubmissionStatus::Accepted,
            "Backend development",
            "Ownership",
            None,
            "95/100",
        );

        assert!(
            message.contains("Предмет: Backend development"),
            "{message}"
        );
        assert!(message.contains("Задание: Ownership"), "{message}");
    }

    #[test]
    fn an_accepted_scored_review_names_the_score() {
        let message = render_review_notification(
            SubmissionStatus::Accepted,
            "Backend development",
            "Ownership",
            Some((4, 5)),
            "Хорошо",
        );

        assert!(message.contains("Балл: 4 из 5"), "{message}");
    }

    #[test]
    fn a_pass_fail_review_names_no_score() {
        let message = render_review_notification(
            SubmissionStatus::Accepted,
            "Backend development",
            "Ownership",
            None,
            "Зачтено",
        );

        assert!(!message.contains("Балл"), "{message}");
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
