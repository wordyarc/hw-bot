use teloxide::ApiError;
use teloxide::prelude::ChatId;

use super::Bot;
use super::HandlerResult;
use crate::app::{AppError, AppResult, ErrorCode};
use crate::telegram::is_invalid_token;
pub(super) use crate::telegram::request_error_kind;

#[derive(Debug, thiserror::Error)]
pub(super) enum BotError {
    #[error(transparent)]
    App(#[from] AppError),
    #[error(transparent)]
    Telegram(#[from] teloxide::RequestError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<teloxide::dispatching::dialogue::InMemStorageError> for BotError {
    fn from(error: teloxide::dispatching::dialogue::InMemStorageError) -> Self {
        Self::Other(error.into())
    }
}

impl BotError {
    pub(super) fn request_error(&self) -> Option<&teloxide::RequestError> {
        match self {
            Self::Telegram(error) => Some(error),
            Self::App(_) | Self::Other(_) => None,
        }
    }

    pub(super) fn is_invalid_token(&self) -> bool {
        self.request_error().is_some_and(is_invalid_token)
    }
}

pub(super) async fn finish_handler(
    bot: &Bot,
    chat_id: ChatId,
    result: HandlerResult,
) -> HandlerResult {
    match result {
        Err(BotError::App(error)) => send_app_error(bot, chat_id, error).await,
        result => result,
    }
}

pub(super) fn optional_stat<T>(result: AppResult<T>, statistic: &'static str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(statistic, error = ?error, "optional statistic is unavailable");
            None
        }
    }
}

pub(super) fn log_handler_error(error: &BotError) {
    if let Some(request) = error.request_error() {
        log_request_error("update", request);
    } else {
        tracing::error!(error = ?error, "Telegram update failed");
    }
}

pub(super) fn log_request_error(operation: &'static str, error: &teloxide::RequestError) {
    let retry_after_seconds = match error {
        teloxide::RequestError::RetryAfter(delay) => Some(delay.seconds()),
        _ => None,
    };
    let api_error = match error {
        teloxide::RequestError::Api(ApiError::Unknown(_) | ApiError::CantParseEntities(_)) => None,
        teloxide::RequestError::Api(error) => Some(tracing::field::debug(error)),
        _ => None,
    };
    tracing::warn!(
        operation,
        kind = request_error_kind(error),
        retry_after_seconds,
        api_error,
        "Telegram request failed"
    );
}

pub(super) async fn send_app_error(bot: &Bot, chat_id: ChatId, error: AppError) -> HandlerResult {
    let code = error.code();
    if code == ErrorCode::InternalError {
        tracing::error!(error = ?error, "Telegram command failed");
    }

    send_error_code(bot, chat_id, code).await
}

pub(super) async fn send_error_code(bot: &Bot, chat_id: ChatId, code: ErrorCode) -> HandlerResult {
    bot.send_message(chat_id, app_error_message(code)).await?;
    Ok(())
}

pub fn app_error_message(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InternalError => "Бот не смог выполнить действие. Повторите попытку позже.",
        ErrorCode::NotFound => "Не удалось найти нужные данные.",
        ErrorCode::InvalidInput => "Проверьте введённые данные и попробуйте ещё раз.",
        ErrorCode::PermissionDenied => "У вас нет доступа к этому действию.",
        ErrorCode::AssignmentUnavailable => "В текущем состоянии задания это действие недоступно.",
        ErrorCode::DeadlineExceeded => "Приём по этому заданию завершён: срок сдачи истёк.",
        ErrorCode::SubmissionClosed => "Приём работ по этому заданию закрыт.",
        ErrorCode::AttemptsExhausted => "Попыток на сдачу больше не осталось.",
        ErrorCode::MakeupAttemptsExhausted => {
            "Попытки досдачи по предмету закончились. Добавить попытку может преподаватель предмета."
        }
        ErrorCode::SubmissionAlreadyAccepted => "Работа уже принята.",
        ErrorCode::SubmissionPendingReview => "Предыдущая работа ещё на проверке.",
        ErrorCode::SubmissionAlreadyReviewed => "Эту сдачу уже проверил другой преподаватель.",
        ErrorCode::AlreadyRegistered => "Вы уже зарегистрированы.",
        ErrorCode::ConcurrentModification => "Данные успели измениться. Повторите действие.",
        ErrorCode::InviteInvalid => {
            "Регистрация по этой ссылке недоступна. Обратитесь к преподавателю за новой ссылкой."
        }
        ErrorCode::FileTooLarge => "Файл слишком большой. Пришлите файл поменьше.",
        ErrorCode::FileLimitReached => {
            "Лимит вложений достигнут. Удалите одно, прежде чем добавить новое."
        }
        ErrorCode::Overloaded => "Бот сейчас перегружен. Попробуйте снова через минуту.",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::test_support::logs::LogBuffer;

    #[test]
    fn transport_logs_keep_correlation_without_the_raw_telegram_response() {
        let logs = LogBuffer::of(|| {
            let error = teloxide::RequestError::InvalidJson {
                source: Arc::new(serde_json::from_str::<serde_json::Value>("invalid").unwrap_err()),
                raw: "private-comment-and-token".into(),
            };
            tracing::info_span!(
                "telegram_update",
                update_id = 7,
                chat_id = 42,
                action = "review"
            )
            .in_scope(|| {
                log_handler_error(&error.clone().into());
                log_request_error("get_updates", &error);
                log_request_error("set_my_commands", &error);
                let startup = super::super::map_startup_error(error);
                assert!(!format!("{startup:?}: {startup}").contains("private-comment-and-token"));
            });
        });
        let events = logs.events();
        assert_eq!(events.len(), 4);
        for event in events {
            assert_eq!(event["span"]["update_id"], 7);
            assert_eq!(event["span"]["action"], "review");
            assert_eq!(event["fields"]["kind"], "telegram_invalid_response");
        }
        assert!(!logs.text().contains("private-comment-and-token"));
    }

    #[test]
    fn an_api_error_is_named_but_its_description_is_not_logged() {
        let logs = LogBuffer::of(|| {
            log_request_error(
                "get_updates",
                &teloxide::RequestError::Api(ApiError::TerminatedByOtherGetUpdates),
            );
            log_request_error(
                "update",
                &teloxide::RequestError::Api(ApiError::Unknown("private-description".into())),
            );
        });
        let events = logs.events();

        assert_eq!(
            events[0]["fields"]["api_error"],
            "TerminatedByOtherGetUpdates"
        );
        assert!(
            events[1]["fields"].get("api_error").is_none(),
            "{}",
            events[1]
        );
        assert!(!logs.text().contains("private-description"));
    }
}
