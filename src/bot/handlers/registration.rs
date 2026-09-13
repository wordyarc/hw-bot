use crate::bot::Bot;
use chrono::Utc;
use teloxide::types::ChatId;

use super::home::show_home;
use super::input::{ensure_text_limit, require_text};
use super::reset_dialogue;
use crate::app::{ErrorCode, RegisterStudent};
use crate::bot::context::{Deps, TextStep};
use crate::bot::errors::{send_app_error, send_error_code};
use crate::bot::invites::BotInvites;
use crate::bot::state::{BotDialogue, BotState, FlowState};
use crate::bot::views::ask_input;
use crate::bot::{BotServices, HandlerResult};
use crate::domain::{DISPLAY_NAME_MAX_LEN, TelegramUserId};
use crate::invites::VerifiedInvite;

pub(super) async fn start_registration(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: TelegramUserId,
    payload: &str,
    services: &BotServices,
    dialogue: &BotDialogue,
    invites: &BotInvites,
) -> HandlerResult {
    match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(_) => return show_home(bot, chat_id, telegram_user_id, services, dialogue).await,
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return send_app_error(bot, chat_id, error).await,
    }
    if payload.is_empty() {
        return super::home::open_guest(bot, chat_id, dialogue).await;
    }
    // Битая подпись и отозванная ссылка обязаны выглядеть для гостя одинаково.
    let Ok(invite) = invites.decode(payload) else {
        return send_error_code(bot, chat_id, ErrorCode::InviteInvalid).await;
    };
    let group = match services.users.resolve_group_invite(invite).await {
        Ok(group) => group,
        Err(error) => return send_app_error(bot, chat_id, error).await,
    };
    dialogue
        .update(BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
        }))
        .await?;
    ask_input(
        bot,
        chat_id,
        &format!(
            "Регистрация в группу {}.\n\nКак вас называть?",
            group.code().as_str()
        ),
    )
    .await
}

pub(super) async fn submit_name(step: TextStep<'_>, invite: VerifiedInvite) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id,
        deps,
    } = step;
    let Deps {
        services,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(message.chat.id, "Имя не должно быть пустым.")
            .await?;
        return Ok(());
    }
    if !ensure_text_limit(bot, message.chat.id, text, DISPLAY_NAME_MAX_LEN, "Имя").await? {
        return Ok(());
    }

    match services
        .users
        .register_student(RegisterStudent {
            telegram_user_id,
            display_name: text.trim().to_owned(),
            invite,
            registered_at: Utc::now(),
        })
        .await
    {
        Ok(_) => {
            reset_dialogue(dialogue).await?;
            bot.send_message(message.chat.id, "🎉 Регистрация завершена.")
                .await?;
            show_home(bot, message.chat.id, telegram_user_id, services, dialogue).await?;
        }
        Err(error) => send_app_error(bot, message.chat.id, error).await?,
    }

    Ok(())
}
