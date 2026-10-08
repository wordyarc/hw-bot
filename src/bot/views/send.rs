use anyhow::Context;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardMarkup, KeyboardRemove, MessageId};

use crate::bot::{Bot, HandlerResult, texts};
use crate::telegram::text::{MESSAGE_LIMIT, utf16_prefix};

fn message_chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let chunk = utf16_prefix(rest, MESSAGE_LIMIT);
        rest = &rest[chunk.len()..];
        (!chunk.is_empty()).then_some(chunk)
    })
}

pub(crate) async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    send_chunks(bot, chat_id, text, keyboard).await?;
    Ok(())
}

pub(crate) async fn send_screen(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    keyboard: InlineKeyboardMarkup,
) -> HandlerResult<MessageId> {
    Ok(send_chunks(bot, chat_id, text, Some(keyboard))
        .await?
        .context("a screen has no text to carry its keyboard")?)
}

async fn send_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    mut keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult<Option<MessageId>> {
    let mut last = None;
    let mut chunks = message_chunks(&text).peekable();
    while let Some(chunk) = chunks.next() {
        let sent = if chunks.peek().is_none()
            && let Some(keyboard) = keyboard.take()
        {
            bot.send_message(chat_id, chunk)
                .reply_markup(keyboard)
                .await?
        } else {
            bot.send_message(chat_id, chunk).await?
        };
        last = Some(sent.id);
    }
    Ok(last)
}

pub(crate) async fn ask_input(bot: &Bot, chat_id: ChatId, prompt: &str) -> HandlerResult {
    bot.send_message(chat_id, texts::input_prompt(prompt))
        .reply_markup(KeyboardRemove::new())
        .await?;
    Ok(())
}

pub(crate) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, texts::STALE_BUTTON).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_are_split_by_telegram_length_without_losing_content() {
        for (source, expected_chunks) in [
            (String::new(), 0),
            ("я".repeat(MESSAGE_LIMIT), 1),
            ("я🦀a".repeat(MESSAGE_LIMIT), 4),
            ("🦀".repeat(MESSAGE_LIMIT), 2),
            (format!("🕘{}", "a".repeat(MESSAGE_LIMIT - 1)), 2),
        ] {
            let chunks: Vec<_> = message_chunks(&source).collect();
            assert_eq!(chunks.len(), expected_chunks);
            assert!(
                chunks
                    .iter()
                    .all(|chunk| chunk.encode_utf16().count() <= MESSAGE_LIMIT)
            );
            assert_eq!(chunks.concat(), source);
        }
    }
}
