use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;

use super::{Screen, reset_dialogue};
use crate::bot::HandlerResult;
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotState, FlowState};
use crate::bot::views::{review_keyboard, teacher_menu};

pub(super) async fn show_next(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let item = match services.reviews.take_next_from_queue(actor).await {
        Ok(item) => item,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let Some(item) = item else {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, "🎉 Очередь проверки пуста.")
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    };

    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id: item.submission.id(),
            grading: item.assignment.grading(),
            penalty: item.assignment.penalty_at(item.submission.submitted_at()),
        }))
        .await?;
    let summary = format!(
        "{}\n👤 Студент: {}",
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
    );
    let keyboard = review_keyboard(item.submission.id());
    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, "📄 Текст задания:").await?;
    bot.send_message(chat_id, item.assignment.body().as_str())
        .await?;
    bot.send_message(chat_id, "📤 Сдача студента:").await?;
    bot.send_message(chat_id, item.submission.content().as_str())
        .reply_markup(keyboard)
        .await?;
    Ok(())
}
