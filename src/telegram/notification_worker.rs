use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{TelegramClient, TelegramError};
use crate::db::{self, DbError, DbResult};
use crate::domain::SubmissionStatus;
use crate::health::ComponentHealth;

const TELEGRAM_MESSAGE_LIMIT: usize = 4096;
const LAST_ERROR_LIMIT: usize = 1000;

#[derive(Clone, Debug)]
pub struct NotificationWorkerSettings {
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
}

#[derive(Debug, Error)]
pub enum NotificationWorkerError {
    #[error("Telegram credentials were rejected")]
    InvalidCredentials,

    #[error(transparent)]
    Telegram(#[from] TelegramError),

    #[error(transparent)]
    Database(#[from] DbError),
}

#[derive(Clone)]
pub struct NotificationWorker {
    pool: PgPool,
    telegram: TelegramClient,
    settings: NotificationWorkerSettings,
}

impl NotificationWorker {
    pub fn new(
        pool: PgPool,
        telegram: TelegramClient,
        settings: NotificationWorkerSettings,
    ) -> Self {
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
            &notification.review_comment,
        );

        match self
            .telegram
            .send_message(notification.recipient_telegram_user_id, &text)
            .await
        {
            Ok(()) => {
                ensure_claim_updated(
                    db::notifications::mark_delivered(
                        &self.pool,
                        notification.id,
                        notification.lease_token,
                        Utc::now(),
                    )
                    .await?,
                )?;
                Ok(DeliveryOutcome::Delivered)
            }
            Err(error) if error.is_invalid_credentials() => {
                let last_error = bounded_error(&error.to_string());
                ensure_claim_updated(
                    db::notifications::release_after_credentials_failure(
                        &self.pool,
                        notification.id,
                        notification.lease_token,
                        &last_error,
                    )
                    .await?,
                )?;
                Err(NotificationWorkerError::InvalidCredentials)
            }
            Err(error)
                if error.is_permanent()
                    || notification.attempt_count >= self.settings.max_attempts =>
            {
                log_retry_after(&error);
                let last_error = bounded_error(&error.to_string());
                ensure_claim_updated(
                    db::notifications::mark_failed(
                        &self.pool,
                        notification.id,
                        notification.lease_token,
                        Utc::now(),
                        &last_error,
                    )
                    .await?,
                )?;
                Ok(DeliveryOutcome::Failed)
            }
            Err(error) => {
                log_retry_after(&error);
                let retry_after = error
                    .retry_after()
                    .unwrap_or_else(|| self.retry_delay(notification.attempt_count));
                let available_at = Utc::now() + chrono_duration(retry_after)?;
                let last_error = bounded_error(&error.to_string());
                ensure_claim_updated(
                    db::notifications::reschedule(
                        &self.pool,
                        notification.id,
                        notification.lease_token,
                        available_at,
                        &last_error,
                    )
                    .await?,
                )?;
                Ok(DeliveryOutcome::Rescheduled)
            }
        }
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        health: ComponentHealth,
    ) -> Result<(), NotificationWorkerError> {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            result = self.telegram.validate_credentials() => match result {
                Ok(()) => {}
                Err(error) if error.is_invalid_credentials() => {
                    return Err(NotificationWorkerError::InvalidCredentials);
                }
                Err(error) => return Err(error.into()),
            },
        }
        let _ready = health.ready();

        loop {
            if cancellation.is_cancelled() {
                return Ok(());
            }

            match self.deliver_one(Utc::now()).await {
                Ok(DeliveryOutcome::Idle) => {
                    tokio::select! {
                        () = cancellation.cancelled() => return Ok(()),
                        () = tokio::time::sleep(self.settings.poll_interval) => {}
                    }
                }
                Ok(_) => {}
                Err(NotificationWorkerError::InvalidCredentials) => {
                    return Err(NotificationWorkerError::InvalidCredentials);
                }
                Err(error) => {
                    tracing::error!(error = ?error, "notification worker failed to access database");
                    tokio::select! {
                        () = cancellation.cancelled() => return Ok(()),
                        () = tokio::time::sleep(self.settings.poll_interval) => {}
                    }
                }
            }
        }
    }

    fn retry_delay(&self, attempt_count: u32) -> Duration {
        retry_delay(&self.settings, attempt_count)
    }
}

fn log_retry_after(error: &TelegramError) {
    if error.is_rate_limited() {
        tracing::warn!(
            retry_after_seconds = error.retry_after().map(|value| value.as_secs()),
            "Telegram rate limited a notification request"
        );
    }
}

fn render_review_notification(
    outcome: SubmissionStatus,
    subject_name: &str,
    assignment_title: &str,
    review_comment: &str,
) -> String {
    let verdict = match outcome {
        SubmissionStatus::Accepted => "✅ Работа принята.",
        SubmissionStatus::Rejected => "❌ Работа отклонена.",
        SubmissionStatus::PendingReview => unreachable!("outbox contains only completed reviews"),
    };
    let text = format!(
        "{verdict}\n📘 Предмет: {subject_name}\n📚 Задание: {assignment_title}\n\n\
         💬 Комментарий преподавателя:\n{review_comment}"
    );

    truncate_chars(text, TELEGRAM_MESSAGE_LIMIT)
}

fn truncate_chars(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }

    let mut truncated: String = text.chars().take(limit.saturating_sub(3)).collect();
    truncated.push_str("...");
    truncated
}

fn bounded_error(error: &str) -> String {
    error.chars().take(LAST_ERROR_LIMIT).collect()
}

fn retry_delay(settings: &NotificationWorkerSettings, attempt_count: u32) -> Duration {
    let multiplier = 1_u32
        .checked_shl(attempt_count.saturating_sub(1).min(31))
        .unwrap_or(u32::MAX);

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

fn ensure_claim_updated(updated: bool) -> DbResult<()> {
    if updated {
        Ok(())
    } else {
        Err(DbError::Conflict {
            message: "notification lease is no longer owned by this worker",
        })
    }
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
            &"x".repeat(TELEGRAM_MESSAGE_LIMIT * 2),
        );

        assert_eq!(message.chars().count(), TELEGRAM_MESSAGE_LIMIT);
        assert!(message.ends_with("..."));
    }

    #[test]
    fn review_message_names_the_subject_the_assignment_belongs_to() {
        let message = render_review_notification(
            SubmissionStatus::Accepted,
            "Backend development",
            "Ownership",
            "95/100",
        );

        assert!(
            message.contains("Предмет: Backend development"),
            "{message}"
        );
        assert!(message.contains("Задание: Ownership"), "{message}");
    }

    #[test]
    fn retry_delay_grows_and_is_capped() {
        let worker = NotificationWorkerSettings {
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
