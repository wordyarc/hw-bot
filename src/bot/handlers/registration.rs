use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::home::send_home;
use super::input::require_text_within;
use super::reset_dialogue;
use crate::app::{CourseInvitation, ErrorCode, RegisterStudent};
use crate::bot::HandlerResult;
use crate::bot::context::{Chat, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{ask_input, registration_groups_keyboard};
use crate::domain::{DISPLAY_NAME_MAX_LEN, StudentGroup, StudentGroupId, TelegramUserId};
use crate::invites::VerifiedInvite;

pub(super) async fn start_registration(
    chat: Chat<'_>,
    telegram_user_id: TelegramUserId,
    payload: &str,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let services = chat.services();
    let invites = chat.invites();
    match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(registered) => return send_home(chat, &registered.user).await,
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if payload.is_empty() {
        return super::home::open_guest(chat).await;
    }
    // Гость не должен отличать ошибку подписи от отзыва ссылки по ответу бота.
    let Ok(invite) = invites.decode(payload) else {
        return send_error_code(bot, chat_id, ErrorCode::InviteInvalid).await;
    };
    let CourseInvitation { subject, groups } = services.users.resolve_course_invite(invite).await?;
    if let [group] = groups.as_slice() {
        return ask_name(chat, invite, subject.name().as_str(), group).await;
    }

    let sent = bot
        .send_message(
            chat_id,
            texts::registration_group_prompt(subject.name().as_str()),
        )
        .reply_markup(registration_groups_keyboard(&groups))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingRegistrationGroup {
            invite,
            message_id: sent.id,
        }))
        .await?;
    Ok(())
}

pub(super) async fn choose_group(
    chat: Chat<'_>,
    message_id: MessageId,
    group_id: StudentGroupId,
    state: &BotState,
) -> HandlerResult {
    let Some(invite) = state.choosing_registration_group(message_id) else {
        return chat.stale().await;
    };
    let invitation = chat.services().users.resolve_course_invite(invite).await;
    if let Err(error) = &invitation
        && error.code() == ErrorCode::InviteInvalid
    {
        reset_dialogue(chat.dialogue()).await?;
    }
    let CourseInvitation { subject, groups } = invitation?;
    let Some(group) = groups.iter().find(|group| group.id() == group_id) else {
        return chat.stale().await;
    };
    chat.bot()
        .edit_message_reply_markup(chat.chat_id(), message_id)
        .await?;
    ask_name(chat, invite, subject.name().as_str(), group).await
}

async fn ask_name(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    subject: &str,
    group: &StudentGroup,
) -> HandlerResult {
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
            group_id: group.id(),
        }))
        .await?;
    ask_input(
        chat.bot(),
        chat.chat_id(),
        &texts::registration_prompt(subject, group.code().as_str()),
    )
    .await
}

pub(super) async fn submit_name(
    step: TextStep<'_>,
    invite: VerifiedInvite,
    group_id: StudentGroupId,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let telegram_user_id = step.telegram_user_id();
    let dialogue = step.dialogue();
    let services = step.services();

    let Some(text) =
        require_text_within(bot, message, DISPLAY_NAME_MAX_LEN, texts::FIELD_NAME).await?
    else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::NAME_REQUIRED).await?;
        return Ok(());
    }

    let registered = services
        .users
        .register_student(RegisterStudent {
            telegram_user_id,
            display_name: text.trim().to_owned(),
            invite,
            student_group_id: Some(group_id),
            registered_at: Utc::now(),
        })
        .await;
    if let Err(error) = &registered
        && matches!(
            error.code(),
            ErrorCode::InviteInvalid | ErrorCode::AlreadyRegistered
        )
    {
        reset_dialogue(dialogue).await?;
    }
    let registered = registered?;
    reset_dialogue(dialogue).await?;
    bot.send_message(chat_id, texts::REGISTRATION_COMPLETE)
        .await?;
    send_home(step.chat(), &registered.user).await
}
