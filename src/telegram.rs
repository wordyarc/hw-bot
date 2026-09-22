use std::time::Duration;

use anyhow::Context;
use teloxide::{ApiError, RequestError};

pub(crate) mod network;
mod notification_worker;
pub(crate) mod text;

pub use notification_worker::{
    DeliveryOutcome, NotificationWorker, NotificationWorkerError, NotificationWorkerSettings,
};

pub fn new_bot(
    token: impl Into<String>,
    api_base_url: &str,
    timeout: Duration,
) -> anyhow::Result<teloxide::Bot> {
    let api_url = api_base_url
        .parse()
        .context("invalid Telegram API base URL")?;
    let http = reqwest::Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .timeout(timeout)
        .tcp_nodelay(true)
        .build()?;
    Ok(teloxide::Bot::with_client(token, http).set_api_url(api_url))
}

pub(crate) fn is_invalid_token(error: &RequestError) -> bool {
    matches!(error, RequestError::Api(ApiError::InvalidToken))
}

pub(crate) fn request_error_kind(error: &RequestError) -> &'static str {
    match error {
        RequestError::RetryAfter(_) => "telegram_rate_limit",
        RequestError::Network(_) => "telegram_network",
        RequestError::Io(_) => "telegram_io_or_deadline",
        RequestError::InvalidJson { .. } => "telegram_invalid_response",
        RequestError::Api(ApiError::InvalidToken) => "telegram_invalid_credentials",
        RequestError::Api(ApiError::BotBlocked) => "telegram_bot_blocked",
        RequestError::Api(ApiError::MessageNotModified) => "telegram_message_not_modified",
        RequestError::Api(ApiError::Unknown(description)) if is_flood_control(description) => {
            "telegram_rate_limit"
        }
        RequestError::Api(_) => "telegram_api",
        RequestError::MigrateToChatId(_) => "telegram_chat_migrated",
    }
}

fn is_flood_control(description: &str) -> bool {
    description
        .to_ascii_lowercase()
        .starts_with("too many requests")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliveryFailure {
    InvalidCredentials,
    Permanent,
    Retry,
    RateLimited(Option<Duration>),
}

fn delivery_failure(error: &RequestError) -> DeliveryFailure {
    match error {
        RequestError::Api(ApiError::InvalidToken) => DeliveryFailure::InvalidCredentials,
        RequestError::RetryAfter(delay) => DeliveryFailure::RateLimited(Some(delay.duration())),
        // Ответ 429 без retry_after попадает в teloxide как Unknown с текстовым описанием.
        RequestError::Api(ApiError::Unknown(description)) if is_flood_control(description) => {
            DeliveryFailure::RateLimited(None)
        }
        RequestError::Api(ApiError::Unknown(description))
            if matches!(
                description.to_ascii_lowercase().as_str(),
                "request timeout"
                    | "internal server error"
                    | "internal error"
                    | "bad gateway"
                    | "service unavailable"
                    | "gateway timeout"
            ) =>
        {
            DeliveryFailure::Retry
        }
        RequestError::Api(_) | RequestError::MigrateToChatId(_) => DeliveryFailure::Permanent,
        RequestError::Network(_) | RequestError::Io(_) | RequestError::InvalidJson { .. } => {
            DeliveryFailure::Retry
        }
    }
}
