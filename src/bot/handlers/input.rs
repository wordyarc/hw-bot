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
}
