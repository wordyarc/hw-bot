use crate::app::ErrorCode;
use crate::bot::HandlerResult;
use crate::bot::context::ActorChat;
use crate::bot::errors::{BotError, log_handler_error, send_error_code};
use crate::bot::files::{GeneratedFile, TransferError};
use crate::bot::{grade_sheet, texts};
use crate::domain::TeachingCourseId;
use crate::telegram::is_invalid_token;

pub(crate) async fn send_grade_sheet(
    screen: ActorChat<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let sent = screen
        .transfers()
        .send_generated(screen.chat_id(), || produce(screen, course_id))
        .await;
    match sent {
        Ok(true) => Ok(()),
        Ok(false) => {
            screen
                .bot()
                .send_message(screen.chat_id(), texts::GRADE_SHEET_NO_STUDENTS)
                .await?;
            Ok(())
        }
        Err(TransferError::App(error)) => Err(error.into()),
        Err(TransferError::Telegram(error)) if is_invalid_token(&error) => Err(error.into()),
        Err(error) => {
            let code = match error {
                TransferError::Deadline => ErrorCode::Overloaded,
                _ => ErrorCode::InternalError,
            };
            log_handler_error(&BotError::from(error));
            send_error_code(screen.bot(), screen.chat_id(), code).await
        }
    }
}

async fn produce(
    screen: ActorChat<'_>,
    course_id: TeachingCourseId,
) -> Result<Option<GeneratedFile>, TransferError> {
    let Some(sheet) = screen
        .services()
        .courses
        .grade_sheet(screen.actor(), course_id)
        .await?
    else {
        return Ok(None);
    };
    let (now, timezone) = (screen.now(), *screen.timezone());
    let file = tokio::task::spawn_blocking(move || grade_sheet::build(&sheet, now, &timezone))
        .await
        .map_err(|error| TransferError::Build(error.into()))?
        .map_err(|error| TransferError::Build(error.into()))?;

    Ok(Some(file))
}
