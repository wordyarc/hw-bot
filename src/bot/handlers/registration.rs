use chrono::Utc;

use super::home::show_home;
use super::input::{ensure_text_limit, require_text};
use super::reset_dialogue;
use crate::app::{ErrorCode, RegisterStudent};
use crate::bot::HandlerResult;
use crate::bot::context::{Chat, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::ask_input;
use crate::domain::{DISPLAY_NAME_MAX_LEN, TelegramUserId};
use crate::invites::VerifiedInvite;

pub(super) async fn start_registration(
    chat: Chat<'_>,
    telegram_user_id: TelegramUserId,
    payload: &str,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let dialogue = chat.dialogue();
    let services = chat.services();
    let invites = chat.invites();
    match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(_) => return show_home(chat, telegram_user_id).await,
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if payload.is_empty() {
        return super::home::open_guest(chat).await;
    }
    // Битая подпись и отозванная ссылка обязаны выглядеть для гостя одинаково.
    let Ok(invite) = invites.decode(payload) else {
        return send_error_code(bot, chat_id, ErrorCode::InviteInvalid).await;
    };
    let group = services.users.resolve_group_invite(invite).await?;
    dialogue
        .update(BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
        }))
        .await?;
    ask_input(
        bot,
        chat_id,
        &texts::registration_prompt(group.code().as_str()),
    )
    .await
}

pub(super) async fn submit_name(step: TextStep<'_>, invite: VerifiedInvite) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let telegram_user_id = step.telegram_user_id();
    let dialogue = step.dialogue();
    let services = step.services();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::NAME_REQUIRED).await?;
        return Ok(());
    }
    if !ensure_text_limit(bot, chat_id, text, DISPLAY_NAME_MAX_LEN, texts::FIELD_NAME).await? {
        return Ok(());
    }

    services
        .users
        .register_student(RegisterStudent {
            telegram_user_id,
            display_name: text.trim().to_owned(),
            invite,
            registered_at: Utc::now(),
        })
        .await?;
    reset_dialogue(dialogue).await?;
    bot.send_message(chat_id, texts::REGISTRATION_COMPLETE)
        .await?;
    show_home(step.chat(), telegram_user_id).await?;

    Ok(())
}
