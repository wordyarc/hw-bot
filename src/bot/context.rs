use super::Bot;
use teloxide::types::{ChatId, Message};

use crate::app::{Actor, AppError};
use crate::domain::TelegramUserId;

use super::BotServices;
use super::HandlerResult;
use super::invites::BotInvites;
use super::state::BotDialogue;
use super::timezone::BotTimezone;

#[derive(Clone, Copy)]
pub(super) struct Chat<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub deps: Deps<'a>,
}

impl Chat<'_> {
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
    pub services: BotServices,
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
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub actor: Actor,
    pub context: &'a BotContext,
    pub dialogue: &'a BotDialogue,
}

impl<'a> Screen<'a> {
    pub fn chat(self) -> Chat<'a> {
        Chat {
            bot: self.bot,
            chat_id: self.chat_id,
            deps: self.deps(),
        }
    }

    pub async fn stale(self) -> HandlerResult {
        self.chat().stale().await
    }

    pub async fn ask(self, prompt: &str) -> HandlerResult {
        self.chat().ask(prompt).await
    }

    pub async fn fail(self, error: AppError) -> HandlerResult {
        self.chat().fail(error).await
    }

    pub fn new(bot: &'a Bot, chat_id: ChatId, actor: Actor, deps: Deps<'a>) -> Self {
        Self {
            bot,
            chat_id,
            actor,
            context: deps.context,
            dialogue: deps.dialogue,
        }
    }

    pub fn deps(self) -> Deps<'a> {
        Deps {
            context: self.context,
            dialogue: self.dialogue,
        }
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

    pub fn screen(self, actor: Actor) -> Screen<'a> {
        Screen::new(self.bot, self.message.chat.id, actor, self.deps)
    }
}
