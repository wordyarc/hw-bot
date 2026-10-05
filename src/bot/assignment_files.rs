use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::HandlerResult;
use super::context::{Screen, TextStep};
use super::errors::send_error_code;
use super::files::{TransferError, incoming_file};
use super::state::{BotState, FileDeletion, FlowState, ScreenState};
use super::texts;
use super::views::{
    assignment_files_keyboard, confirm_file_deletion_keyboard, send_message_chunks,
    teacher_keyboard,
};
use crate::app::{AddAssignmentFile, DeleteAssignmentFile, ErrorCode};
use crate::domain::{AssignmentId, AssignmentStatus, StoredFileMetadata};

pub(super) async fn submit_file(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
    expected_revision: i64,
    creation_flow: bool,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let services = step.services();
    let Some(incoming) = incoming_file(message) else {
        bot.send_message(chat_id, texts::FILE_PROMPT).await?;
        return Ok(());
    };
    let policy = services.files.policy();
    if incoming
        .declared_size
        .is_some_and(|size| size > policy.max_file_size_bytes)
    {
        send_error_code(bot, chat_id, ErrorCode::FileTooLarge).await?;
        return Ok(());
    }
    let transfer = step.transfers().acquire().await?;
    let content = match transfer
        .download(&incoming.file_id, policy.max_file_size_bytes)
        .await
    {
        Ok(content) => content,
        Err(TransferError::TooLarge) => {
            send_error_code(bot, chat_id, ErrorCode::FileTooLarge).await?;
            return Ok(());
        }
        Err(_) => {
            tracing::warn!(
                kind = "file_download_failed",
                "Telegram file download failed"
            );
            bot.send_message(chat_id, texts::FILE_DOWNLOAD_FAILED)
                .await?;
            return Ok(());
        }
    };
    if incoming
        .declared_size
        .is_some_and(|declared| declared != content.len())
    {
        bot.send_message(chat_id, texts::FILE_SIZE_MISMATCH).await?;
        return Ok(());
    }
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let result = services
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id,
            actor: screen.actor(),
            expected_revision,
            presentation: incoming.presentation,
            filename: incoming.filename,
            mime_type: incoming.mime_type,
            telegram_file_id: Some(incoming.file_id.0),
            content,
        })
        .await;
    drop(transfer);
    match result {
        Ok(_) => {
            bot.send_message(chat_id, texts::FILE_ADDED)
                .reply_markup(teacher_keyboard())
                .await?;
            show(screen, assignment_id, creation_flow).await?;
        }
        Err(error) if error.code() == ErrorCode::ConcurrentModification => {
            step.chat().fail(error).await?;
            show(screen, assignment_id, creation_flow).await?;
        }
        Err(error) => {
            return Err(error.into());
        }
    }
    Ok(())
}

pub(super) async fn ask_upload(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(files) = state.files_screen(message_id) else {
        return screen.stale().await;
    };
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingAssignmentFile {
            assignment_id: files.assignment_id,
            expected_revision: files.revision,
            creation_flow: files.creation_flow,
        }))
        .await?;
    screen.ask(texts::FILE_PROMPT).await
}

pub(super) async fn open(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
    index: usize,
) -> HandlerResult {
    let Some(files) = state.files_screen(message_id) else {
        return screen.stale().await;
    };
    let Some(file_id) = files.file_ids.get(index).copied() else {
        return screen.stale().await;
    };
    let file = screen
        .services()
        .files
        .open_assignment_file_for_teacher(screen.actor(), files.assignment_id, file_id)
        .await?;
    screen
        .transfers()
        .send(screen.chat_id(), &screen.services().files, &file)
        .await?;
    Ok(())
}

pub(super) async fn request_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
    index: usize,
) -> HandlerResult {
    let Some(files) = state.files_screen(message_id) else {
        return screen.stale().await;
    };
    let Some(file_id) = files.file_ids.get(index).copied() else {
        return screen.stale().await;
    };
    let deletion = FileDeletion {
        assignment_id: files.assignment_id,
        file_id,
        expected_revision: files.revision,
        creation_flow: files.creation_flow,
    };
    if files.requires_confirmation {
        let confirmation = screen
            .bot()
            .send_message(screen.chat_id(), texts::FILE_DELETE_CONFIRM)
            .reply_markup(confirm_file_deletion_keyboard())
            .await?;
        screen
            .dialogue()
            .update(BotState::Flow(
                FlowState::ConfirmingAssignmentFileDeletion {
                    message_id: confirmation.id,
                    deletion,
                },
            ))
            .await?;
    } else {
        delete(screen, deletion).await?;
    }
    Ok(())
}

pub(super) async fn confirm_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(deletion) = state.confirming_file_deletion(message_id) else {
        return screen.stale().await;
    };
    delete(screen, deletion).await
}

pub(super) async fn cancel_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(deletion) = state.confirming_file_deletion(message_id) else {
        return screen.stale().await;
    };
    show(screen, deletion.assignment_id, deletion.creation_flow).await
}

pub(super) async fn show(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    creation_flow: bool,
) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let card = services
        .files
        .list_assignment_files_for_teacher(actor, assignment_id)
        .await?;
    let policy = services.files.policy();
    let mut lines = vec![if card.files.is_empty() {
        texts::NO_FILES.to_owned()
    } else {
        texts::FILES_HEADER.to_owned()
    }];
    for file in &card.files {
        lines.push(String::new());
        lines.push(texts::file_entry(
            file.position(),
            file.filename().as_str(),
            &texts::format_file_size(file.size_bytes()),
        ));
    }
    lines.push(String::new());
    lines.push(texts::files_used(
        card.files.len(),
        policy.max_files_per_assignment,
    ));
    let text = lines.join("\n");
    let can_add = card.files.len() < policy.max_files_per_assignment;
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, texts::CHOOSE_ACTION)
        .reply_markup(assignment_files_keyboard(
            &card.assignment,
            &card.files,
            can_add,
            creation_flow,
        ))
        .await?;
    dialogue
        .update(BotState::Screen(ScreenState::ManagingAssignmentFiles {
            message_id: message.id,
            assignment_id,
            revision: card.revision,
            file_ids: card.files.iter().map(StoredFileMetadata::id).collect(),
            deletion_requires_confirmation: card.assignment.status() != AssignmentStatus::Draft,
            creation_flow,
        }))
        .await?;
    Ok(())
}

async fn delete(screen: Screen<'_>, deletion: FileDeletion) -> HandlerResult {
    let FileDeletion {
        assignment_id,
        file_id,
        expected_revision,
        creation_flow,
    } = deletion;
    match screen
        .services()
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id,
            file_id,
            actor: screen.actor(),
            expected_revision,
        })
        .await
    {
        Ok(_) => {
            screen
                .bot()
                .send_message(screen.chat_id(), texts::FILE_DELETED)
                .await?;
        }
        Err(error) => screen.fail(error).await?,
    }
    show(screen, assignment_id, creation_flow).await
}
