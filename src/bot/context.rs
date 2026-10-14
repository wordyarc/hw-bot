use chrono::{DateTime, Utc};
use teloxide::types::{ChatId, Message};

use super::Bot;
use super::HandlerResult;
use super::clock::Clock;
use super::files::FileTransfers;
use super::invites::BotInvites;
use super::state::BotDialogue;
use super::timezone::BotTimezone;
use crate::app::{Actor, AppError, ApplicationContainer, ErrorCode};
use crate::domain::TelegramUserId;

#[derive(Clone)]
pub(super) struct BotContext {
    pub services: ApplicationContainer,
    pub timezone: BotTimezone,
    pub clock: Clock,
    pub invites: BotInvites,
    pub transfers: FileTransfers,
}

#[derive(Clone, Copy)]
pub(super) struct Deps<'a> {
    pub context: &'a BotContext,
    pub dialogue: &'a BotDialogue,
}

#[derive(Clone, Copy)]
pub(super) struct Chat<'a> {
    bot: &'a Bot,
    chat_id: ChatId,
    deps: Deps<'a>,
}

impl<'a> Chat<'a> {
    pub fn new(bot: &'a Bot, chat_id: ChatId, deps: Deps<'a>) -> Self {
        Self { bot, chat_id, deps }
    }

    pub fn bot(&self) -> &'a Bot {
        self.bot
    }

    pub fn chat_id(&self) -> ChatId {
        self.chat_id
    }

    pub fn dialogue(&self) -> &'a BotDialogue {
        self.deps.dialogue
    }

    pub fn services(&self) -> &'a ApplicationContainer {
        &self.deps.context.services
    }

    pub fn timezone(&self) -> &'a BotTimezone {
        &self.deps.context.timezone
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.deps.context.clock.now()
    }

    pub fn invites(&self) -> &'a BotInvites {
        &self.deps.context.invites
    }

    pub fn transfers(&self) -> &'a FileTransfers {
        &self.deps.context.transfers
    }

    pub async fn resolve(
        self,
        telegram_user_id: TelegramUserId,
    ) -> HandlerResult<Option<Screen<'a>>> {
        match self.services().users.resolve_actor(telegram_user_id).await {
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

    pub async fn ask_dated(self, prompt: &str) -> HandlerResult<DateTime<Utc>> {
        super::views::ask_input_dated(self.bot, self.chat_id, prompt).await
    }

    pub async fn fail(self, error: AppError) -> HandlerResult {
        super::errors::send_app_error(self.bot, self.chat_id, error).await
    }
}

#[derive(Clone, Copy)]
pub(super) struct Screen<'a> {
    chat: Chat<'a>,
    actor: Actor,
}

impl<'a> Screen<'a> {
    pub fn chat(&self) -> Chat<'a> {
        self.chat
    }

    pub fn actor(&self) -> Actor {
        self.actor
    }

    pub fn bot(&self) -> &'a Bot {
        self.chat.bot
    }

    pub fn chat_id(&self) -> ChatId {
        self.chat.chat_id
    }

    pub fn dialogue(&self) -> &'a BotDialogue {
        self.chat.dialogue()
    }

    pub fn services(&self) -> &'a ApplicationContainer {
        self.chat.services()
    }

    pub fn timezone(&self) -> &'a BotTimezone {
        self.chat.timezone()
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.chat.now()
    }

    pub fn invites(&self) -> &'a BotInvites {
        self.chat.invites()
    }

    pub fn transfers(&self) -> &'a FileTransfers {
        self.chat.transfers()
    }

    pub async fn stale(self) -> HandlerResult {
        self.chat.stale().await
    }

    pub async fn ask(self, prompt: &str) -> HandlerResult {
        self.chat.ask(prompt).await
    }

    pub async fn fail(self, error: AppError) -> HandlerResult {
        self.chat.fail(error).await
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextStep<'a> {
    bot: &'a Bot,
    message: &'a Message,
    telegram_user_id: TelegramUserId,
    deps: Deps<'a>,
}

impl<'a> TextStep<'a> {
    pub fn new(
        bot: &'a Bot,
        message: &'a Message,
        telegram_user_id: TelegramUserId,
        deps: Deps<'a>,
    ) -> Self {
        Self {
            bot,
            message,
            telegram_user_id,
            deps,
        }
    }

    pub fn bot(&self) -> &'a Bot {
        self.bot
    }

    pub fn message(&self) -> &'a Message {
        self.message
    }

    pub fn chat_id(&self) -> ChatId {
        self.message.chat.id
    }

    pub fn telegram_user_id(&self) -> TelegramUserId {
        self.telegram_user_id
    }

    pub fn dialogue(&self) -> &'a BotDialogue {
        self.deps.dialogue
    }

    pub fn services(&self) -> &'a ApplicationContainer {
        &self.deps.context.services
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.deps.context.clock.now()
    }

    pub fn transfers(&self) -> &'a FileTransfers {
        &self.deps.context.transfers
    }

    pub fn chat(&self) -> Chat<'a> {
        Chat::new(self.bot, self.message.chat.id, self.deps)
    }

    pub async fn resolve(self) -> HandlerResult<Option<Screen<'a>>> {
        self.chat().resolve(self.telegram_user_id).await
    }
}
