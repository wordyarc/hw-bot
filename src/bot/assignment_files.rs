use crate::app::{Actor, AddAssignmentFile, DeleteAssignmentFile, ErrorCode};
use crate::domain::{
    AssignmentId, AssignmentStatus, StoredFileId, StoredFileMetadata, TelegramUserId,
};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Bot, Requester};
use teloxide::types::{ChatId, Message, MessageId};

use super::callbacks::{FlowAction, ScreenAction};
use super::errors::send_app_error;
use super::files::{FileTransfers, incoming_file};
use super::state::{BotDialogue, BotState, FlowState, ScreenState};
use super::views::{
    ask_input, assignment_files_keyboard, confirm_file_deletion_keyboard, format_file_size,
    send_message_chunks, send_stale_button, teacher_keyboard,
};
use super::{BotServices, HandlerResult};

pub(super) struct CallbackContext<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub message_id: MessageId,
    pub state: &'a BotState,
    pub actor: Actor,
    pub dialogue: &'a BotDialogue,
    pub services: &'a BotServices,
    pub transfers: &'a FileTransfers,
}

struct Workflow<'a> {
    bot: &'a Bot,
    chat_id: ChatId,
    actor: Actor,
    dialogue: &'a BotDialogue,
    services: &'a BotServices,
}

pub(super) async fn handle_message(
    bot: &Bot,
    message: &Message,
    state: &BotState,
    dialogue: &BotDialogue,
    services: &BotServices,
    transfers: &FileTransfers,
    telegram_user_id: TelegramUserId,
) -> HandlerResult {
    match state {
        BotState::Screen(ScreenState::ManagingAssignmentFiles { .. }) => {
            bot.send_message(
                message.chat.id,
                "Выберите действие кнопкой под списком вложений.",
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentFile {
            assignment_id,
            expected_revision,
            creation_flow,
        }) => {
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
            let content = match transfers
                .download(&incoming.file_id, policy.max_file_size_bytes)
                .await
            {
                Ok(content) => content,
                Err(error) => {
                    tracing::warn!(error = ?error, "Telegram file download failed");
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
            let Some(actor) = crate::bot::handlers::resolve_actor(
                bot,
                message.chat.id,
                telegram_user_id,
                services,
            )
            .await?
            else {
                return Ok(());
            };
            match services
                .files
                .add_assignment_file(AddAssignmentFile {
                    assignment_id: *assignment_id,
                    actor,
                    expected_revision: *expected_revision,
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
                    show(
                        bot,
                        message.chat.id,
                        actor,
                        *assignment_id,
                        *creation_flow,
                        services,
                        dialogue,
                    )
                    .await?;
                }
                Err(error) if error.code() == ErrorCode::ConcurrentModification => {
                    drop(transfer_permit);
                    send_app_error(bot, message.chat.id, error).await?;
                    show(
                        bot,
                        message.chat.id,
                        actor,
                        *assignment_id,
                        *creation_flow,
                        services,
                        dialogue,
                    )
                    .await?;
                }
                Err(error) => {
                    drop(transfer_permit);
                    send_app_error(bot, message.chat.id, error).await?;
                }
            }
        }
        BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. }) => {
            bot.send_message(message.chat.id, "Подтвердите удаление кнопкой выше.")
                .await?;
        }
        _ => unreachable!("not an assignment file state"),
    }
    Ok(())
}

pub(super) async fn handle_screen_action(
    context: CallbackContext<'_>,
    action: ScreenAction,
) -> HandlerResult {
    let CallbackContext {
        bot,
        chat_id,
        message_id,
        state,
        actor,
        dialogue,
        services,
        transfers,
    } = context;
    let workflow = Workflow {
        bot,
        chat_id,
        actor,
        dialogue,
        services,
    };
    match action {
        ScreenAction::AddFile => {
            let Some((assignment_id, revision, creation_flow)) =
                state.managing_assignment_files(message_id)
            else {
                return send_stale_button(bot, chat_id).await;
            };
            dialogue
                .update(BotState::Flow(FlowState::AwaitingAssignmentFile {
                    assignment_id,
                    expected_revision: revision,
                    creation_flow,
                }))
                .await?;
            ask_input(bot, chat_id, "Пришлите файл или фотографию.").await?;
        }
        ScreenAction::OpenFile(index) => {
            let Some((assignment_id, file_id)) = current_file(index, message_id, state) else {
                send_stale_button(bot, chat_id).await?;
                return Ok(());
            };
            let _permit = transfers.acquire().await?;
            match services
                .files
                .open_assignment_file_for_teacher(actor, assignment_id, file_id)
                .await
            {
                Ok(file) => transfers.send(chat_id, file).await?,
                Err(error) => send_app_error(bot, chat_id, error).await?,
            }
        }
        ScreenAction::DeleteFile(index) => {
            let Some((assignment_id, file_id)) = current_file(index, message_id, state) else {
                send_stale_button(bot, chat_id).await?;
                return Ok(());
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
                let confirmation = bot
                    .send_message(chat_id, crate::bot::texts::FILE_DELETE_CONFIRM)
                    .reply_markup(confirm_file_deletion_keyboard())
                    .await?;
                dialogue
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
                workflow
                    .delete(assignment_id, file_id, *revision, *creation_flow)
                    .await?;
            }
        }
        ScreenAction::FinishFileCreation
        | ScreenAction::Publish(_)
        | ScreenAction::Archive(_)
        | ScreenAction::ChangeDeadline(_)
        | ScreenAction::RotateGroupInvite => unreachable!("not an assignment file action"),
    }
    Ok(())
}

pub(super) async fn handle_flow_action(
    context: CallbackContext<'_>,
    action: FlowAction,
) -> HandlerResult {
    let CallbackContext {
        bot,
        chat_id,
        message_id,
        state,
        actor,
        dialogue,
        services,
        transfers: _,
    } = context;
    let workflow = Workflow {
        bot,
        chat_id,
        actor,
        dialogue,
        services,
    };

    match action {
        FlowAction::ConfirmFileDeletion => {
            let Some((assignment_id, file_id, expected_revision, creation_flow)) =
                state.confirming_file_deletion(message_id)
            else {
                return send_stale_button(bot, chat_id).await;
            };
            workflow
                .delete(assignment_id, file_id, expected_revision, creation_flow)
                .await?;
        }
        FlowAction::CancelFileDeletion => {
            let Some((assignment_id, _, _, creation_flow)) =
                state.confirming_file_deletion(message_id)
            else {
                return send_stale_button(bot, chat_id).await;
            };
            show(
                bot,
                chat_id,
                actor,
                assignment_id,
                creation_flow,
                services,
                dialogue,
            )
            .await?;
        }
        FlowAction::ConfirmInviteRotation
        | FlowAction::CancelInviteRotation
        | FlowAction::Review { .. }
        | FlowAction::MaxAttempts(_)
        | FlowAction::Grading(_)
        | FlowAction::LatePolicy(_)
        | FlowAction::ReviewScore(_)
        | FlowAction::Deadline(_)
        | FlowAction::SelectStudent(_)
        | FlowAction::ExtraAttempts(_) => unreachable!("not an assignment file action"),
    }
    Ok(())
}

pub(super) async fn show(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    assignment_id: AssignmentId,
    creation_flow: bool,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
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

impl Workflow<'_> {
    async fn delete(
        &self,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
        expected_revision: i64,
        creation_flow: bool,
    ) -> HandlerResult {
        match self
            .services
            .files
            .delete_assignment_file(DeleteAssignmentFile {
                assignment_id,
                file_id,
                actor: self.actor,
                expected_revision,
            })
            .await
        {
            Ok(_) => {
                self.bot
                    .send_message(self.chat_id, "Вложение удалено.")
                    .await?;
            }
            Err(error) => send_app_error(self.bot, self.chat_id, error).await?,
        }
        show(
            self.bot,
            self.chat_id,
            self.actor,
            assignment_id,
            creation_flow,
            self.services,
            self.dialogue,
        )
        .await
    }
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
