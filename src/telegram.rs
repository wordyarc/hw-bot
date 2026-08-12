use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::TelegramUserId;

mod notification_worker;

pub use notification_worker::{DeliveryOutcome, NotificationWorker, NotificationWorkerSettings};

#[derive(Clone)]
pub struct TelegramClient {
    http: reqwest::Client,
    api_base_url: String,
    bot_token: String,
}

impl TelegramClient {
    pub fn new(
        api_base_url: impl Into<String>,
        bot_token: impl Into<String>,
        request_timeout: Duration,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(request_timeout)
                .build()?,
            api_base_url: api_base_url.into().trim_end_matches('/').to_owned(),
            bot_token: bot_token.into(),
        })
    }

    pub async fn send_message(
        &self,
        recipient: TelegramUserId,
        text: &str,
    ) -> Result<(), TelegramError> {
        let url = format!("{}/bot{}/sendMessage", self.api_base_url, self.bot_token);
        let response = self
            .http
            .post(url)
            .json(&SendMessageRequest {
                chat_id: recipient.as_i64(),
                text,
            })
            .send()
            .await
            .map_err(|error| TelegramError::Retryable {
                message: if error.is_timeout() {
                    "request timed out".to_owned()
                } else {
                    "transport error".to_owned()
                },
                retry_after: None,
                rate_limited: false,
            })?;

        let status = response.status();
        let body = response.json::<TelegramResponse>().await;

        match body {
            Ok(body) if status.is_success() && body.ok => Ok(()),
            Ok(body) => Err(classify_error(
                body.error_code.unwrap_or(status.as_u16()),
                body.description
                    .unwrap_or_else(|| "request was rejected".to_owned()),
                body.parameters
                    .and_then(|parameters| parameters.retry_after)
                    .map(Duration::from_secs),
            )),
            Err(_) if status.is_server_error() || matches!(status.as_u16(), 401 | 408 | 429) => {
                Err(TelegramError::Retryable {
                    message: format!("HTTP {status}"),
                    retry_after: None,
                    rate_limited: status.as_u16() == 429,
                })
            }
            Err(_) if status.is_client_error() => Err(TelegramError::Permanent {
                message: format!("HTTP {status}"),
            }),
            Err(_) => Err(TelegramError::Retryable {
                message: "malformed response".to_owned(),
                retry_after: None,
                rate_limited: false,
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum TelegramError {
    #[error("Telegram API temporarily unavailable: {message}")]
    Retryable {
        message: String,
        retry_after: Option<Duration>,
        rate_limited: bool,
    },

    #[error("Telegram rejected the message: {message}")]
    Permanent { message: String },
}

impl TelegramError {
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Retryable { retry_after, .. } => *retry_after,
            Self::Permanent { .. } => None,
        }
    }

    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent { .. })
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Self::Retryable {
                rate_limited: true,
                ..
            }
        )
    }
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse {
    ok: bool,
    description: Option<String>,
    error_code: Option<u16>,
    parameters: Option<ResponseParameters>,
}

#[derive(Debug, Deserialize)]
struct ResponseParameters {
    retry_after: Option<u64>,
}

fn classify_error(
    error_code: u16,
    message: String,
    retry_after: Option<Duration>,
) -> TelegramError {
    if matches!(error_code, 401 | 408 | 429) || error_code >= 500 {
        TelegramError::Retryable {
            message,
            retry_after,
            rate_limited: error_code == 429,
        }
    } else {
        TelegramError::Permanent { message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_telegram_429_without_retry_after() {
        let error = classify_error(429, "too many requests".to_owned(), None);

        assert!(error.is_rate_limited());
        assert_eq!(error.retry_after(), None);
    }
}
