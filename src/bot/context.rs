use super::Bot;
use teloxide::types::{ChatId, Message};

use crate::app::Actor;
use crate::domain::TelegramUserId;

use super::BotServices;
use super::invites::BotInvites;
use super::state::BotDialogue;
use super::timezone::BotTimezone;

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
    pub fn screen(self, actor: Actor) -> Screen<'a> {
        Screen::new(self.bot, self.message.chat.id, actor, self.deps)
    }
}
