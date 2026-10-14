use anyhow::Context;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardMarkup, KeyboardRemove, MessageId};

use crate::bot::{Bot, HandlerResult, texts};
use crate::telegram::text::{MESSAGE_LIMIT, utf16_prefix};

fn message_chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let mut chunk = utf16_prefix(rest, MESSAGE_LIMIT);
        if chunk.len() < rest.len()
            && let Some(newline) = chunk.rfind('\n')
            && chunk[..newline].contains(|character: char| !character.is_whitespace())
        {
            chunk = &chunk[..=newline];
        }
        rest = &rest[chunk.len()..];
        (!chunk.is_empty()).then_some(chunk)
    })
}

pub(crate) async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    send_chunks(bot, chat_id, text, keyboard).await?;
    Ok(())
}

pub(crate) async fn send_screen(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    keyboard: InlineKeyboardMarkup,
) -> HandlerResult<MessageId> {
    Ok(send_chunks(bot, chat_id, text, Some(keyboard))
        .await?
        .context("a screen has no text to carry its keyboard")?)
}

async fn send_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    mut keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult<Option<MessageId>> {
    let mut last = None;
    let mut chunks = message_chunks(text).peekable();
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
    ask_input_message(bot, chat_id, prompt).await?;
    Ok(())
}

pub(crate) async fn ask_input_message(
    bot: &Bot,
    chat_id: ChatId,
    prompt: &str,
) -> HandlerResult<MessageId> {
    let sent = bot
        .send_message(chat_id, texts::input_prompt(prompt))
        .reply_markup(KeyboardRemove::new())
        .await?;
    Ok(sent.id)
}

pub(crate) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, texts::STALE_BUTTON).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::domain::USER_TEXT_MAX_LEN;

    fn chunks(source: &str) -> Vec<&str> {
        message_chunks(source).collect()
    }

    fn units(text: &str) -> usize {
        text.encode_utf16().count()
    }

    #[test]
    fn an_empty_text_has_no_chunks() {
        assert!(chunks("").is_empty());
    }

    #[test]
    fn a_text_within_the_limit_is_one_chunk() {
        let source = "я".repeat(MESSAGE_LIMIT);

        assert_eq!(chunks(&source), [source.as_str()]);
    }

    #[test]
    fn the_limit_is_counted_in_utf16_units() {
        for (pattern, expected) in [("🦀", 2), ("я🦀a", 4)] {
            let source = pattern.repeat(MESSAGE_LIMIT);

            let chunks = chunks(&source);

            assert_eq!(chunks.len(), expected, "{pattern}");
            assert!(
                chunks.iter().all(|chunk| units(chunk) <= MESSAGE_LIMIT),
                "{pattern}"
            );
            assert_eq!(chunks.concat(), source, "{pattern}");
        }
    }

    #[test]
    fn a_surrogate_pair_at_the_limit_moves_to_the_next_chunk_whole() {
        let source = format!("🕘{}", "a".repeat(MESSAGE_LIMIT - 1));

        let chunks = chunks(&source);

        assert_eq!(units(chunks[0]), MESSAGE_LIMIT);
        assert_eq!(chunks[1], "a");
    }

    #[test]
    fn a_multiline_text_is_split_at_the_ends_of_lines() {
        let roster = "12. Иванова Анна — сдано 7 из 12\n".repeat(400);

        let chunks = chunks(&roster);

        assert_eq!(chunks.len(), 4);
        assert!(
            chunks[..chunks.len() - 1]
                .iter()
                .all(|chunk| chunk.ends_with('\n'))
        );
        assert_eq!(chunks.concat(), roster);
    }

    #[test]
    fn a_line_longer_than_the_limit_is_cut_at_the_limit() {
        let long_line = "b".repeat(MESSAGE_LIMIT + 5);
        let source = format!("Список\n{long_line}");

        let chunks = chunks(&source);

        assert_eq!(chunks, ["Список\n", &long_line[..MESSAGE_LIMIT], "bbbbb"]);
    }

    #[test]
    fn a_blank_head_stays_with_the_text_after_it() {
        let long_line = "b".repeat(MESSAGE_LIMIT + 5);
        for head in ["\n", " \n\n"] {
            let source = format!("{head}{long_line}");

            let chunks = chunks(&source);

            assert_eq!(chunks.len(), 2, "{head:?}");
            assert!(chunks[0].starts_with(head), "{head:?}");
            assert_eq!(chunks.concat(), source, "{head:?}");
        }
    }

    fn user_text() -> impl Strategy<Value = String> {
        let run = (
            prop::sample::select(vec!['a', 'я', '🦀', ' ', '\n', '\t']),
            1..=MESSAGE_LIMIT,
        );
        prop::collection::vec(run, 1..6).prop_filter_map("a blank user text", |runs| {
            let text: String = runs
                .into_iter()
                .flat_map(|(character, count)| std::iter::repeat_n(character, count))
                .take(USER_TEXT_MAX_LEN)
                .collect();
            let text = text.trim();
            (!text.is_empty()).then(|| text.to_owned())
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn chunks_of_a_screen_with_user_texts_keep_the_text_fit_the_limit_and_are_never_blank(
            parts in prop::collection::vec(user_text(), 1..4),
        ) {
            let source = parts.join("\n");

            let chunks = chunks(&source);

            prop_assert_eq!(chunks.concat(), source.as_str());
            for chunk in chunks {
                prop_assert!(units(chunk) <= MESSAGE_LIMIT);
                prop_assert!(!chunk.trim().is_empty());
            }
        }
    }
}
