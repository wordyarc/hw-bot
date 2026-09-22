use teloxide::prelude::ChatId;

use super::Bot;
use super::HandlerResult;
use crate::app::{AppError, ErrorCode};

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
        self.request_error()
            .is_some_and(crate::telegram::is_invalid_token)
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

pub(super) fn log_handler_error(error: &BotError) {
    if let Some(request) = error.request_error() {
        log_request_error("update", request);
    } else {
        tracing::error!(error = ?error, "Telegram update failed");
    }
}

pub(super) use crate::telegram::request_error_kind;

pub(super) fn log_request_error(operation: &'static str, error: &teloxide::RequestError) {
    let retry_after_seconds = match error {
        teloxide::RequestError::RetryAfter(delay) => Some(delay.seconds()),
        _ => None,
    };
    tracing::warn!(
        operation,
        kind = request_error_kind(error),
        retry_after_seconds,
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
        ErrorCode::InternalError => "У бота что-то пошло не так. Попробуйте позже.",
        ErrorCode::NotFound => "Не удалось найти нужные данные.",
        ErrorCode::InvalidInput => {
            "Не удалось выполнить действие. Проверьте данные и попробуйте ещё раз."
        }
        ErrorCode::PermissionDenied => "У вас нет доступа к этому действию.",
        ErrorCode::AssignmentUnavailable => "Сейчас это действие с заданием выполнить нельзя.",
        ErrorCode::DeadlineExceeded => "Срок сдачи прошёл. Приём работ по этому заданию закрыт.",
        ErrorCode::SubmissionClosed => "Приём работ по этому заданию закрыт.",
        ErrorCode::AttemptsExhausted => "Попыток на сдачу больше не осталось.",
        ErrorCode::SubmissionAlreadyAccepted => "Работа уже принята.",
        ErrorCode::SubmissionPendingReview => "Предыдущая работа ещё на проверке.",
        ErrorCode::SubmissionAlreadyReviewed => "Эту сдачу уже проверил другой преподаватель.",
        ErrorCode::AlreadyRegistered => "Вы уже зарегистрированы.",
        ErrorCode::ConcurrentModification => "За это время данные изменились. Попробуйте ещё раз.",
        ErrorCode::InviteInvalid => {
            "Эта ссылка-приглашение не работает. Попросите у преподавателя новую."
        }
        ErrorCode::FileTooLarge => "Файл слишком большой. Пришлите файл поменьше.",
        ErrorCode::FileLimitReached => {
            "Больше вложений добавить нельзя. Сначала удалите одно из имеющихся."
        }
        ErrorCode::Overloaded => "Бот сейчас перегружен. Попробуйте снова через минуту.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_logs_keep_correlation_without_the_raw_telegram_response() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct Writer(Arc<Mutex<Vec<u8>>>);
        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let buffer = Writer(Arc::default());
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
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
        let bytes = buffer.0.lock().unwrap();
        let events: Vec<_> = serde_json::Deserializer::from_slice(&bytes)
            .into_iter::<serde_json::Value>()
            .map(Result::unwrap)
            .collect();
        assert_eq!(events.len(), 4);
        for event in events {
            assert_eq!(event["span"]["update_id"], 7);
            assert_eq!(event["span"]["action"], "review");
            assert_eq!(event["fields"]["kind"], "telegram_invalid_response");
        }
        assert!(!String::from_utf8_lossy(&bytes).contains("private-comment-and-token"));
    }

    #[test]
    fn common_submission_errors_have_student_facing_text() {
        assert_eq!(
            app_error_message(ErrorCode::SubmissionPendingReview),
            "Предыдущая работа ещё на проверке."
        );
        assert_eq!(
            app_error_message(ErrorCode::SubmissionAlreadyReviewed),
            "Эту сдачу уже проверил другой преподаватель."
        );
    }
}
