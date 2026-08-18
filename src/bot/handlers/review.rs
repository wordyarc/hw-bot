use super::{
    Deps, TextStep, ask_review_comment, confirm_score, ensure_text_limit, parse_score,
    require_text, resolve_actor, show_home,
};
use crate::app::ErrorCode;
use crate::app::ReviewSubmission;
use crate::bot::views::teacher_keyboard;
use crate::domain::{ReviewDecision, SubmissionId, USER_TEXT_MAX_LEN};
use chrono::Utc;
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
        bot.send_message(chat_id, crate::bot::texts::REVIEW_QUEUE_EMPTY)
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

pub(super) async fn submit_score_text(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    max_score: u16,
    penalty: u16,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id: _,
        deps,
    } = step;
    let Deps {
        services: _,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(score) = parse_score(text, max_score) else {
        bot.send_message(
            message.chat.id,
            format!("Введите целое число от 0 до {max_score}."),
        )
        .await?;
        return Ok(());
    };
    confirm_score(bot, message.chat.id, score, penalty, max_score).await?;
    ask_review_comment(
        bot,
        message.chat.id,
        submission_id,
        ReviewDecision::Accept,
        Some(score),
        dialogue,
    )
    .await?;

    Ok(())
}

pub(super) async fn submit_comment(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    score: Option<u16>,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id,
        deps,
    } = step;
    let Deps {
        services,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if !ensure_text_limit(bot, message.chat.id, text, USER_TEXT_MAX_LEN, "Комментарий").await?
    {
        return Ok(());
    }
    let Some(reviewer) = resolve_actor(bot, message.chat.id, telegram_user_id, services).await?
    else {
        return Ok(());
    };
    let result = services
        .reviews
        .review_submission(ReviewSubmission {
            submission_id,
            actor: reviewer,
            decision,
            score,
            comment: text.to_owned(),
            reviewed_at: Utc::now(),
        })
        .await;

    match result {
        Ok(_) => {
            reset_dialogue(dialogue).await?;
            bot.send_message(
                message.chat.id,
                "✅ Решение сохранено, студент получит уведомление.",
            )
            .reply_markup(teacher_keyboard())
            .await?;
        }
        Err(error) if error.code() == ErrorCode::SubmissionAlreadyReviewed => {
            reset_dialogue(dialogue).await?;
            send_app_error(bot, message.chat.id, error).await?;
            show_home(bot, message.chat.id, telegram_user_id, services).await?;
        }
        Err(error) => send_app_error(bot, message.chat.id, error).await?,
    }

    Ok(())
}
