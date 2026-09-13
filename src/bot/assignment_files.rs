use crate::app::{AddAssignmentFile, DeleteAssignmentFile, ErrorCode};
use crate::domain::{AssignmentId, AssignmentStatus, StoredFileId, StoredFileMetadata};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::HandlerResult;
use super::context::{Screen, TextStep};
use super::errors::send_app_error;
use super::files::{FileTransfers, incoming_file};
use super::state::{BotState, FlowState, ScreenState};
use super::views::{
    ask_input, assignment_files_keyboard, confirm_file_deletion_keyboard, format_file_size,
    send_message_chunks, send_stale_button, teacher_keyboard,
};

pub(super) async fn submit_file(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
    expected_revision: i64,
    creation_flow: bool,
    transfers: &FileTransfers,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id,
        deps,
    } = step;
    let services = deps.services;
    let Some(incoming) = incoming_file(message) else {
        bot.send_message(message.chat.id, "Пришлите файл или фотографию.")
            .await?;
        return Ok(());
    };
    let policy = services.files.policy();
    if incoming
        .declared_size
        .is_some_and(|size| size > policy.max_file_size_bytes)
    {
        bot.send_message(message.chat.id, "Файл превышает допустимый размер.")
            .await?;
        return Ok(());
    }
    let transfer_permit = transfers.acquire().await?;
    let content = match transfer_permit
        .run(transfers.download(&incoming.file_id, policy.max_file_size_bytes))
        .await
    {
        Ok(content) => content,
        Err(_) => {
            tracing::warn!(
                kind = "file_download_failed",
                "Telegram file download failed"
            );
            bot.send_message(message.chat.id, crate::bot::texts::FILE_DOWNLOAD_FAILED)
                .await?;
            return Ok(());
        }
    };
    if incoming
        .declared_size
        .is_some_and(|declared| declared != content.len())
    {
        bot.send_message(
            message.chat.id,
            "Telegram передал файл с неверным размером. Попробуйте ещё раз.",
        )
        .await?;
        return Ok(());
    }
    let Some(actor) =
        crate::bot::handlers::resolve_actor(bot, message.chat.id, telegram_user_id, services)
            .await?
    else {
        return Ok(());
    };
    match services
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id,
            actor,
            expected_revision,
            presentation: incoming.presentation,
            filename: incoming.filename,
            mime_type: incoming.mime_type,
            content,
        })
        .await
    {
        Ok(_) => {
            drop(transfer_permit);
            bot.send_message(message.chat.id, "Вложение добавлено.")
                .reply_markup(teacher_keyboard())
                .await?;
            show(step.screen(actor), assignment_id, creation_flow).await?;
        }
        Err(error) if error.code() == ErrorCode::ConcurrentModification => {
            drop(transfer_permit);
            send_app_error(bot, message.chat.id, error).await?;
            show(step.screen(actor), assignment_id, creation_flow).await?;
        }
        Err(error) => {
            drop(transfer_permit);
            send_app_error(bot, message.chat.id, error).await?;
        }
    }
    Ok(())
}

pub(super) async fn ask_upload(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((assignment_id, revision, creation_flow)) =
        state.managing_assignment_files(message_id)
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    screen
        .dialogue
        .update(BotState::Flow(FlowState::AwaitingAssignmentFile {
            assignment_id,
            expected_revision: revision,
            creation_flow,
        }))
        .await?;
    ask_input(screen.bot, screen.chat_id, "Пришлите файл или фотографию.").await
}

pub(super) async fn open(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
    index: usize,
    transfers: &FileTransfers,
) -> HandlerResult {
    let Some((assignment_id, file_id)) = current_file(index, message_id, state) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    let transfer = transfers.acquire().await?;
    transfer
        .run(async {
            match screen
                .services
                .files
                .open_assignment_file_for_teacher(screen.actor, assignment_id, file_id)
                .await
            {
                Ok(file) => transfers.send(screen.chat_id, file).await?,
                Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
            }
            Ok(())
        })
        .await
}

pub(super) async fn request_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
    index: usize,
) -> HandlerResult {
    let Some((assignment_id, file_id)) = current_file(index, message_id, state) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    let BotState::Screen(ScreenState::ManagingAssignmentFiles {
        revision,
        deletion_requires_confirmation,
        creation_flow,
        ..
    }) = state
    else {
        unreachable!();
    };
    if *deletion_requires_confirmation {
        let confirmation = screen
            .bot
            .send_message(screen.chat_id, crate::bot::texts::FILE_DELETE_CONFIRM)
            .reply_markup(confirm_file_deletion_keyboard())
            .await?;
        screen
            .dialogue
            .update(BotState::Flow(
                FlowState::ConfirmingAssignmentFileDeletion {
                    message_id: confirmation.id,
                    assignment_id,
                    file_id,
                    expected_revision: *revision,
                    creation_flow: *creation_flow,
                },
            ))
            .await?;
    } else {
        delete(screen, assignment_id, file_id, *revision, *creation_flow).await?;
    }
    Ok(())
}

pub(super) async fn confirm_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((assignment_id, file_id, expected_revision, creation_flow)) =
        state.confirming_file_deletion(message_id)
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    delete(
        screen,
        assignment_id,
        file_id,
        expected_revision,
        creation_flow,
    )
    .await
}

pub(super) async fn cancel_deletion(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((assignment_id, _, _, creation_flow)) = state.confirming_file_deletion(message_id)
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    show(screen, assignment_id, creation_flow).await
}

pub(super) async fn show(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    creation_flow: bool,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let card = match services
        .files
        .list_assignment_files_for_teacher(actor, assignment_id)
        .await
    {
        Ok(card) => card,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let policy = services.files.policy();
    let mut text = if card.files.is_empty() {
        String::from("Вложений нет.")
    } else {
        String::from("Вложения задания:\n")
    };
    for file in &card.files {
        text.push_str(&format!(
            "\n{}. {} · {}\n",
            file.position(),
            file.filename().as_str(),
            format_file_size(file.size_bytes()),
        ));
    }
    text.push_str(&format!(
        "\nИспользовано: {} из {}.",
        card.files.len(),
        policy.max_files_per_assignment,
    ));
    let can_add = card.files.len() < policy.max_files_per_assignment;
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, "Выберите действие:")
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

async fn delete(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    file_id: StoredFileId,
    expected_revision: i64,
    creation_flow: bool,
) -> HandlerResult {
    match screen
        .services
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id,
            file_id,
            actor: screen.actor,
            expected_revision,
        })
        .await
    {
        Ok(_) => {
            screen
                .bot
                .send_message(screen.chat_id, "Вложение удалено.")
                .await?;
        }
        Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
    }
    show(screen, assignment_id, creation_flow).await
}

fn current_file(
    index: usize,
    message_id: MessageId,
    state: &BotState,
) -> Option<(AssignmentId, StoredFileId)> {
    let BotState::Screen(ScreenState::ManagingAssignmentFiles {
        message_id: current_message_id,
        assignment_id,
        file_ids,
        ..
    }) = state
    else {
        return None;
    };
    (*current_message_id == message_id)
        .then(|| file_ids.get(index).copied())
        .flatten()
        .map(|file_id| (*assignment_id, file_id))
}
