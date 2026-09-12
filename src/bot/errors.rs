use teloxide::prelude::{Bot, ChatId, Requester};

use crate::app::{AppError, ErrorCode};

use super::HandlerResult;

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
