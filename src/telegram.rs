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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use teloxide::types::{ChatId, Seconds};

    use super::*;

    const TEMPORARY_SERVER_ERRORS: [&str; 6] = [
        "Request Timeout",
        "Internal Server Error",
        "Internal Error",
        "Bad Gateway",
        "Service Unavailable",
        "Gateway Timeout",
    ];

    const FLOOD_CONTROL: &str = "Too Many Requests: retry later";

    fn unknown(description: &str) -> RequestError {
        RequestError::Api(ApiError::Unknown(description.to_owned()))
    }

    fn in_every_case(description: &str) -> [RequestError; 3] {
        [
            unknown(description),
            unknown(&description.to_ascii_lowercase()),
            unknown(&description.to_ascii_uppercase()),
        ]
    }

    fn network() -> RequestError {
        reqwest::Client::new()
            .get("not a url")
            .build()
            .unwrap_err()
            .into()
    }

    fn io() -> RequestError {
        RequestError::Io(Arc::new(std::io::ErrorKind::TimedOut.into()))
    }

    fn invalid_json() -> RequestError {
        RequestError::InvalidJson {
            source: Arc::new(serde_json::from_str::<serde_json::Value>("{").unwrap_err()),
            raw: "{".into(),
        }
    }

    fn migrated() -> RequestError {
        RequestError::MigrateToChatId(ChatId(-1_001_234_567_890))
    }

    #[test]
    fn every_temporary_server_error_is_retried_whatever_its_case() {
        for error in TEMPORARY_SERVER_ERRORS.into_iter().flat_map(in_every_case) {
            assert_eq!(
                delivery_failure(&error),
                DeliveryFailure::Retry,
                "{error:?}"
            );
        }
    }

    #[test]
    fn transport_failures_are_retried() {
        for error in [network(), io(), invalid_json()] {
            assert_eq!(
                delivery_failure(&error),
                DeliveryFailure::Retry,
                "{error:?}"
            );
        }
    }

    #[test]
    fn flood_control_waits_for_retry_after_only_when_telegram_names_it() {
        assert_eq!(
            delivery_failure(&RequestError::RetryAfter(Seconds::from_seconds(5))),
            DeliveryFailure::RateLimited(Some(Duration::from_secs(5)))
        );
        for error in in_every_case(FLOOD_CONTROL) {
            assert_eq!(
                delivery_failure(&error),
                DeliveryFailure::RateLimited(None),
                "{error:?}"
            );
        }
    }

    #[test]
    fn an_invalid_token_is_told_apart_from_a_rejected_notification() {
        assert_eq!(
            delivery_failure(&RequestError::Api(ApiError::InvalidToken)),
            DeliveryFailure::InvalidCredentials
        );
    }

    #[test]
    fn other_api_errors_and_a_migrated_chat_are_permanent() {
        for error in [
            RequestError::Api(ApiError::BotBlocked),
            RequestError::Api(ApiError::ChatNotFound),
            RequestError::Api(ApiError::UserDeactivated),
            RequestError::Api(ApiError::MessageNotModified),
            unknown("Bad Request: message text is empty"),
            unknown("Forbidden: bot was kicked from the group chat"),
            migrated(),
        ] {
            assert_eq!(
                delivery_failure(&error),
                DeliveryFailure::Permanent,
                "{error:?}"
            );
        }
    }

    #[test]
    fn every_request_error_is_logged_under_its_own_category() {
        let cases = [
            (
                RequestError::RetryAfter(Seconds::from_seconds(5)),
                "telegram_rate_limit",
            ),
            (network(), "telegram_network"),
            (io(), "telegram_io_or_deadline"),
            (invalid_json(), "telegram_invalid_response"),
            (
                RequestError::Api(ApiError::InvalidToken),
                "telegram_invalid_credentials",
            ),
            (
                RequestError::Api(ApiError::BotBlocked),
                "telegram_bot_blocked",
            ),
            (
                RequestError::Api(ApiError::MessageNotModified),
                "telegram_message_not_modified",
            ),
            (RequestError::Api(ApiError::ChatNotFound), "telegram_api"),
            (unknown("Bad Gateway"), "telegram_api"),
            (migrated(), "telegram_chat_migrated"),
        ];
        let flood_control =
            in_every_case(FLOOD_CONTROL).map(|error| (error, "telegram_rate_limit"));

        for (error, kind) in cases.into_iter().chain(flood_control) {
            assert_eq!(request_error_kind(&error), kind, "{error:?}");
        }
    }
}
