use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::domain::{SubmissionId, SubmissionStatus};

use super::HandlerResult;

const MESSAGE_LIMIT: usize = 4096;

pub(super) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Мои задания",
        "menu:assignments",
    )]])
}

pub(super) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || "Проверить следующую".to_owned(),
        |pending| format!("Проверить следующую ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(review_label, "review:next")],
        vec![InlineKeyboardButton::callback("Мои курсы", "menu:courses")],
    ])
}

pub(super) fn group_keyboard(group_codes: &[String]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        group_codes
            .iter()
            .enumerate()
            .map(|(index, code)| {
                vec![InlineKeyboardButton::callback(
                    code.clone(),
                    format!("register_group:{index}"),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "Принять",
            format!("review:accept:{}", submission_id.as_uuid()),
        ),
        InlineKeyboardButton::callback(
            "Отклонить",
            format!("review:reject:{}", submission_id.as_uuid()),
        ),
    ]])
}

pub(super) fn format_deadline(deadline: chrono::DateTime<Utc>) -> String {
    deadline.format("%d.%m.%Y %H:%M UTC").to_string()
}

pub(super) fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "на проверке",
        SubmissionStatus::Accepted => "принята",
        SubmissionStatus::Rejected => "отклонена",
    }
}

pub(super) fn button_label(label: &str) -> String {
    let truncated = label.chars().count() > 60;
    let mut label: String = label.chars().take(60).collect();
    if truncated {
        label.push_str("...");
    }
    label
}

fn message_chunks(text: String) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;

    for character in text.chars() {
        if current_len == MESSAGE_LIMIT {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push(character);
        current_len += 1;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

pub(super) async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    let chunks = message_chunks(text);
    let last = chunks.len().saturating_sub(1);

    for (index, chunk) in chunks.into_iter().enumerate() {
        if index == last
            && let Some(keyboard) = keyboard.clone()
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

pub(super) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, "Эта кнопка устарела. Откройте /menu ещё раз.")
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_messages_are_split_without_losing_content() {
        let source = "я".repeat(MESSAGE_LIMIT * 2 + 1);
        let chunks = message_chunks(source.clone());

        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MESSAGE_LIMIT)
        );
        assert_eq!(chunks.concat(), source);
    }

    #[test]
    fn group_keyboard_keeps_callback_data_short() {
        let keyboard = group_keyboard(&["б9122-02.03.21".to_owned(), "магистратура-2".to_owned()]);
        let json = serde_json::to_value(keyboard).unwrap();

        assert_eq!(
            json["inline_keyboard"][0][0]["callback_data"],
            "register_group:0"
        );
        assert_eq!(
            json["inline_keyboard"][1][0]["callback_data"],
            "register_group:1"
        );
    }

    #[test]
    fn button_labels_are_truncated_only_when_needed() {
        assert_eq!(button_label("Мои задания"), "Мои задания");
        assert_eq!(button_label(&"я".repeat(61)).chars().count(), 63);
        assert!(button_label(&"я".repeat(61)).ends_with("..."));
    }
}
