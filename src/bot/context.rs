use teloxide::prelude::Bot;
use teloxide::types::{ChatId, Message};

use crate::app::Actor;
use crate::domain::TelegramUserId;

use super::BotServices;
use super::invites::BotInvites;
use super::state::BotDialogue;
use super::timezone::BotTimezone;

#[derive(Clone, Copy)]
pub(super) struct Deps<'a> {
    pub services: &'a BotServices,
    pub dialogue: &'a BotDialogue,
    pub timezone: &'a BotTimezone,
    pub invites: &'a BotInvites,
}

#[derive(Clone, Copy)]
pub(super) struct Screen<'a> {
    pub bot: &'a Bot,
    pub chat_id: ChatId,
    pub actor: Actor,
    pub services: &'a BotServices,
    pub dialogue: &'a BotDialogue,
    pub timezone: &'a BotTimezone,
    pub invites: &'a BotInvites,
}

impl<'a> Screen<'a> {
    pub fn new(bot: &'a Bot, chat_id: ChatId, actor: Actor, deps: Deps<'a>) -> Self {
        Self {
            bot,
            chat_id,
            actor,
            services: deps.services,
            dialogue: deps.dialogue,
            timezone: deps.timezone,
            invites: deps.invites,
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
