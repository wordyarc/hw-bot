use super::Bot;
use teloxide::prelude::ChatId;

use crate::app::{AppError, ErrorCode};

use super::HandlerResult;

pub(super) fn log_handler_error(error: &anyhow::Error) {
    if let Some(request) = error.downcast_ref::<teloxide::RequestError>() {
        let kind = match request {
            teloxide::RequestError::RetryAfter(retry_after) => {
                tracing::warn!(
                    kind = "telegram_rate_limit",
                    retry_after_seconds = retry_after.seconds(),
                    "Telegram update failed"
                );
                return;
            }
            teloxide::RequestError::Network(_) => "telegram_network",
            teloxide::RequestError::Io(_) => "telegram_io_or_deadline",
            teloxide::RequestError::InvalidJson { .. } => "telegram_invalid_response",
            teloxide::RequestError::Api(_) => "telegram_api",
            teloxide::RequestError::MigrateToChatId(_) => "telegram_chat_migrated",
        };
        tracing::error!(kind, "Telegram update failed");
    } else {
        tracing::error!(error = ?error, "Telegram update failed");
    }
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
        ErrorCode::InternalError => "Произошла внутренняя ошибка. Попробуйте позже.",
        ErrorCode::NotFound => "Запрошенные данные не найдены.",
        ErrorCode::InvalidInput => {
            "Не удалось выполнить действие. Проверьте данные и попробуйте ещё раз."
        }
        ErrorCode::PermissionDenied => "Это действие вам недоступно.",
        ErrorCode::AssignmentUnavailable => "Это действие сейчас недоступно.",
        ErrorCode::DeadlineExceeded => "Дедлайн задания уже прошёл.",
        ErrorCode::SubmissionClosed => "Приём работ по этому заданию закрыт.",
        ErrorCode::AttemptsExhausted => "Доступные попытки закончились.",
        ErrorCode::SubmissionAlreadyAccepted => "Работа уже принята.",
        ErrorCode::SubmissionPendingReview => "Предыдущая работа ещё на проверке.",
        ErrorCode::SubmissionAlreadyReviewed => "Эту сдачу уже проверил другой преподаватель.",
        ErrorCode::AlreadyRegistered => "Вы уже зарегистрированы.",
        ErrorCode::ConcurrentModification => "Данные уже изменились. Повторите действие.",
        ErrorCode::InviteInvalid => {
            "Приглашение недействительно. Попросите у преподавателя новую ссылку."
        }
        ErrorCode::FileTooLarge => "Файл превышает допустимый размер.",
        ErrorCode::FileLimitReached => "Достигнут лимит вложений для задания.",
        ErrorCode::Overloaded => "Сервис сейчас занят. Повторите через минуту.",
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
            .in_scope(|| log_handler_error(&error.into()));
        });
        let bytes = buffer.0.lock().unwrap();
        let event: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(event["span"]["update_id"], 7);
        assert_eq!(event["span"]["action"], "review");
        assert_eq!(event["fields"]["kind"], "telegram_invalid_response");
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
