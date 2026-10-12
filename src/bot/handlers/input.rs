use teloxide::types::{ChatId, Message};

use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::texts;
use crate::domain::{Grading, MAX_ATTEMPTS};

pub(super) async fn ensure_text_limit(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    max_len: usize,
    field: &str,
) -> HandlerResult<bool> {
    if text.trim().chars().count() <= max_len {
        return Ok(true);
    }
    bot.send_message(chat_id, texts::text_too_long(field, max_len))
        .await?;
    Ok(false)
}

pub(super) async fn require_text<'a>(
    bot: &Bot,
    message: &'a Message,
) -> HandlerResult<Option<&'a str>> {
    let Some(text) = message.text() else {
        bot.send_message(message.chat.id, texts::TEXT_REQUIRED)
            .await?;
        return Ok(None);
    };
    Ok(Some(text))
}

pub(super) fn parse_max_score(text: &str) -> Option<Grading> {
    Grading::scored(parse_number(text)?).ok()
}

pub(super) fn parse_number(text: &str) -> Option<u16> {
    parse_u16(text.trim())
}

pub(crate) fn parse_u16(value: &str) -> Option<u16> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

pub(super) fn parse_positive_attempts(text: &str) -> Option<u16> {
    parse_number(text).filter(|value| (1..=MAX_ATTEMPTS).contains(value))
}

pub(super) fn looks_like_command(text: &str) -> bool {
    let Some(name) = text
        .split_whitespace()
        .next()
        .and_then(|word| word.strip_prefix('/'))
    else {
        return false;
    };
    let name = name.split_once('@').map_or(name, |(name, _)| name);
    name.chars().next().is_some_and(char::is_alphabetic)
        && name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_input_uses_strict_format_and_limits() {
        assert_eq!(parse_positive_attempts("1"), Some(1));
        assert_eq!(
            parse_positive_attempts(&MAX_ATTEMPTS.to_string()),
            Some(MAX_ATTEMPTS)
        );
        assert!(parse_positive_attempts("0").is_none());
        assert!(parse_positive_attempts("+1").is_none());
        assert!(parse_positive_attempts("32768").is_none());
    }

    #[test]
    fn a_leading_slash_word_is_a_command_even_when_the_bot_does_not_know_it() {
        for text in [
            "/stop",
            "/Cancel",
            "/cancel@other_bot",
            "/отмена",
            "/tasks now",
        ] {
            assert!(looks_like_command(text), "{text}");
        }
    }

    #[test]
    fn a_slash_before_a_number_starts_text_rather_than_a_command() {
        for text in ["/5 баллов: не хватает тестов", "/10", "/2_3"] {
            assert!(!looks_like_command(text), "{text}");
        }
    }

    #[test]
    fn a_path_or_plain_text_is_not_a_command() {
        for text in [
            "/usr/bin/python",
            "pull/1",
            "/",
            "https://example.test",
            "",
            "  ",
        ] {
            assert!(!looks_like_command(text), "{text}");
        }
    }
}
