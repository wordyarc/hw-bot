use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardMarkup, KeyboardRemove};

use crate::bot::{Bot, HandlerResult, texts};
use crate::telegram::text::MESSAGE_LIMIT;

fn message_chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut boundaries = text.char_indices().step_by(MESSAGE_LIMIT).peekable();
    std::iter::from_fn(move || {
        let (start, _) = boundaries.next()?;
        let end = boundaries.peek().map_or(text.len(), |&(index, _)| index);
        Some(&text[start..end])
    })
}

pub(crate) async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    mut keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    let mut chunks = message_chunks(&text).peekable();
    while let Some(chunk) = chunks.next() {
        if chunks.peek().is_none()
            && let Some(keyboard) = keyboard.take()
        {
            bot.send_message(chat_id, chunk)
                .reply_markup(keyboard)
                .await?;
        } else {
            bot.send_message(chat_id, chunk).await?;
        }
    }
    Ok(())
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
    fn long_messages_are_split_without_losing_content() {
        for source in [
            String::new(),
            "я".repeat(MESSAGE_LIMIT),
            "я🦀a".repeat(MESSAGE_LIMIT),
        ] {
            let chunks: Vec<_> = message_chunks(&source).collect();
            assert_eq!(chunks.len(), source.chars().count().div_ceil(MESSAGE_LIMIT));
            assert!(
                chunks
                    .iter()
                    .all(|chunk| chunk.chars().count() <= MESSAGE_LIMIT)
            );
            assert_eq!(chunks.concat(), source);
        }
    }
}
