use teloxide::types::{ChatId, Message};

use super::Bot;
use super::HandlerResult;
use super::invites::BotInvites;
use super::state::BotDialogue;
use super::timezone::BotTimezone;
use crate::app::{Actor, AppError, ApplicationContainer, ErrorCode};
use crate::domain::TelegramUserId;

#[derive(Clone, Copy)]
pub(super) struct Chat<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub deps: Deps<'a>,
}

impl<'a> Chat<'a> {
    pub async fn resolve(
        self,
        telegram_user_id: TelegramUserId,
    ) -> HandlerResult<Option<Screen<'a>>> {
        match self
            .deps
            .context
            .services
            .users
            .resolve_actor(telegram_user_id)
            .await
        {
            Ok(actor) => Ok(Some(Screen { chat: self, actor })),
            Err(error) if error.code() == ErrorCode::NotFound => {
                super::handlers::home::open_guest(self).await?;
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn stale(self) -> HandlerResult {
        super::views::send_stale_button(self.bot, self.chat_id).await
    }

    pub async fn ask(self, prompt: &str) -> HandlerResult {
        super::views::ask_input(self.bot, self.chat_id, prompt).await
    }

    pub async fn fail(self, error: AppError) -> HandlerResult {
        super::errors::send_app_error(self.bot, self.chat_id, error).await
    }
}

#[derive(Clone)]
pub(super) struct BotContext {
    pub services: ApplicationContainer,
    pub timezone: BotTimezone,
    pub invites: BotInvites,
}

#[derive(Clone, Copy)]
pub(super) struct Deps<'a> {
    pub context: &'a BotContext,
    pub dialogue: &'a BotDialogue,
}

#[derive(Clone, Copy)]
pub(super) struct Screen<'a> {
    pub chat: Chat<'a>,
    pub actor: Actor,
}

impl<'a> std::ops::Deref for Screen<'a> {
    type Target = Chat<'a>;

    fn deref(&self) -> &Self::Target {
        &self.chat
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextStep<'a> {
    pub bot: &'a Bot,
    pub message: &'a Message,
    pub telegram_user_id: TelegramUserId,
    pub deps: Deps<'a>,
}

impl<'a> TextStep<'a> {
    pub fn chat(self) -> Chat<'a> {
        Chat {
            bot: self.bot,
            chat_id: self.message.chat.id,
            deps: self.deps,
        }
    }

    pub async fn resolve(self) -> HandlerResult<Option<Screen<'a>>> {
        self.chat().resolve(self.telegram_user_id).await
    }
}
