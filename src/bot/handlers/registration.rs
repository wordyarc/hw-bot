use chrono::Utc;
use teloxide::prelude::Requester;

use super::{Deps, TextStep, ensure_text_limit, require_text, reset_dialogue, show_home};
use crate::app::RegisterStudent;
use crate::bot::HandlerResult;
use crate::bot::errors::send_app_error;
use crate::domain::DISPLAY_NAME_MAX_LEN;
use crate::invites::VerifiedInvite;

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
            show_home(bot, message.chat.id, telegram_user_id, services).await?;
        }
        Err(error) => send_app_error(bot, message.chat.id, error).await?,
    }

    Ok(())
}
