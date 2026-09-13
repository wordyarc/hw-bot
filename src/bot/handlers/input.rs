use crate::bot::Bot;
use teloxide::types::{ChatId, Message};

use crate::bot::HandlerResult;
use crate::bot::timezone::BotTimezone;
use crate::domain::{Grading, LatePolicy, MAX_ATTEMPTS};

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
    bot.send_message(
        chat_id,
        format!("{field} не должно быть длиннее {max_len} символов."),
    )
    .await?;
    Ok(false)
}

pub(super) async fn require_text<'a>(
    bot: &Bot,
    message: &'a Message,
) -> HandlerResult<Option<&'a str>> {
    let Some(text) = message.text() else {
        bot.send_message(message.chat.id, "Пришлите текстовое сообщение.")
            .await?;
        return Ok(None);
    };
    Ok(Some(text))
}

pub(super) fn parse_late_policy(text: &str) -> Option<LatePolicy> {
    let mut parts = text.split_whitespace();
    let mut next = || parse_number(parts.next()?);
    let (period_days, penalty, max_periods) = (next()?, next()?, next()?);
    parts
        .next()
        .is_none()
        .then(|| LatePolicy::new(period_days, penalty, max_periods).ok())
        .flatten()
}

pub(super) fn parse_max_score(text: &str) -> Option<Grading> {
    Grading::scored(parse_number(text)?).ok()
}

pub(super) fn parse_number(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

pub(super) fn parse_positive_attempts(text: &str) -> Option<u16> {
    let value = parse_number(text)?;
    (value > 0 && value <= MAX_ATTEMPTS).then_some(value)
}

pub(super) async fn send_deadline_format_error(
    bot: &Bot,
    chat_id: ChatId,
    timezone: &BotTimezone,
) -> HandlerResult {
    bot.send_message(
        chat_id,
        format!(
            "Не удалось разобрать дату. Используйте ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ в поясе {}.",
            timezone.suffix()
        ),
    )
    .await?;
    Ok(())
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
