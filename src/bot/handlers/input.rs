use std::borrow::Cow;

use teloxide::types::{Message, MessageEntity, MessageEntityKind, MessageEntityRef};

use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::texts;
use crate::domain::{Grading, MAX_ATTEMPTS};

pub(super) async fn require_text_within<'a>(
    bot: &Bot,
    message: &'a Message,
    max_len: usize,
    field: &str,
) -> HandlerResult<Option<Cow<'a, str>>> {
    let Some(text) = require_text(bot, message).await? else {
        return Ok(None);
    };
    match within_limit(text, max_len, field) {
        Ok(text) => Ok(Some(text)),
        Err(refusal) => {
            bot.send_message(message.chat.id, refusal).await?;
            Ok(None)
        }
    }
}

fn within_limit<'a>(
    text: Cow<'a, str>,
    max_len: usize,
    field: &str,
) -> Result<Cow<'a, str>, String> {
    if text.trim().chars().count() <= max_len {
        return Ok(text);
    }
    Err(match text {
        Cow::Borrowed(_) => texts::text_too_long(field, max_len),
        Cow::Owned(_) => texts::text_with_links_too_long(field, max_len),
    })
}

pub(super) async fn require_text<'a>(
    bot: &Bot,
    message: &'a Message,
) -> HandlerResult<Option<Cow<'a, str>>> {
    let Some(text) = message.text() else {
        bot.send_message(message.chat.id, texts::TEXT_REQUIRED)
            .await?;
        return Ok(None);
    };
    Ok(Some(reveal_links(
        text,
        message.entities().unwrap_or_default(),
    )))
}

fn reveal_links<'a>(text: &'a str, entities: &[MessageEntity]) -> Cow<'a, str> {
    let mut revealed = String::new();
    let mut copied = 0;
    for entity in MessageEntityRef::parse(text, entities) {
        let MessageEntityKind::TextLink { url } = entity.kind() else {
            continue;
        };
        if entity.start() < copied
            || entity.text().trim_end_matches('/') == url.as_str().trim_end_matches('/')
        {
            continue;
        }
        revealed.push_str(&text[copied..entity.end()]);
        revealed.push_str(" (");
        revealed.push_str(url.as_str());
        revealed.push(')');
        copied = entity.end();
    }
    if copied == 0 {
        return Cow::Borrowed(text);
    }
    revealed.push_str(&text[copied..]);
    Cow::Owned(revealed)
}

pub(super) fn parse_max_score(text: &str) -> Option<Grading> {
    Grading::scored(parse_number(text)?).ok()
}

pub(super) fn parse_number(text: &str) -> Option<u16> {
    parse_u16(text.trim())
}

pub(super) fn parse_step_number(text: &str) -> Option<u16> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if negative {
        return Some(0);
    }
    Some(digits.parse().unwrap_or(u16::MAX))
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

    fn link(url: &str, offset: usize, length: usize) -> MessageEntity {
        MessageEntity::text_link(url.parse().unwrap(), offset, length)
    }

    #[test]
    fn a_hidden_link_is_written_out_after_its_text() {
        let entities = [link("https://github.com/org/repo/pull/7", 4, 2)];

        assert_eq!(
            reveal_links("Мой PR готов", &entities),
            "Мой PR (https://github.com/org/repo/pull/7) готов"
        );
    }

    #[test]
    fn link_offsets_are_counted_in_utf16_units() {
        let entities = [
            link("https://example.test/a", 3, 1),
            link("https://example.test/b", 7, 1),
        ];

        assert_eq!(
            reveal_links("🙂 a и b", &entities),
            "🙂 a (https://example.test/a) и b (https://example.test/b)"
        );
    }

    #[test]
    fn a_link_that_shows_its_own_address_is_not_repeated() {
        let entities = [link("https://example.test", 0, 20)];

        assert!(matches!(
            reveal_links("https://example.test", &entities),
            Cow::Borrowed("https://example.test")
        ));
    }

    #[test]
    fn text_without_hidden_links_is_kept_as_is() {
        let entities = [MessageEntity::bold(0, 3)];

        assert!(matches!(
            reveal_links("Мой PR", &entities),
            Cow::Borrowed("Мой PR")
        ));
    }

    #[test]
    fn a_text_over_the_limit_only_with_its_links_names_the_links() {
        let entities = [link("https://example.test/a-hidden-link", 0, 1)];
        let revealed = reveal_links("abc", &entities);

        assert_eq!(
            within_limit(revealed, 3, "Поле"),
            Err(texts::text_with_links_too_long("Поле", 3))
        );
        assert_eq!(
            within_limit(Cow::Borrowed("abcd"), 3, "Поле"),
            Err(texts::text_too_long("Поле", 3))
        );
        assert_eq!(
            within_limit(Cow::Borrowed(" abc "), 3, "Поле"),
            Ok(Cow::Borrowed(" abc "))
        );
    }

    #[test]
    fn an_integer_outside_u16_stays_outside_every_step_range() {
        assert_eq!(parse_step_number(" 7 "), Some(7));
        assert_eq!(parse_step_number("-1"), Some(0));
        assert_eq!(parse_step_number("70000"), Some(u16::MAX));
        for text in ["", "-", "+5", "1.5", "семь", "- 1"] {
            assert_eq!(parse_step_number(text), None, "{text}");
        }
    }

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
