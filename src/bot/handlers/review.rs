use super::home::show_home;
use super::input::{ensure_text_limit, parse_number, require_text};
use super::reset_dialogue;
use crate::app::{AppError, ErrorCode, ReviewQueueItem, ReviewSubmission};
use crate::bot::Bot;
use crate::bot::context::{Deps, Screen, TextStep};
use crate::bot::views::teacher_keyboard;
use crate::domain::{DomainError, ReviewDecision, SubmissionId, USER_TEXT_MAX_LEN};
use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::ChatId;

use crate::bot::HandlerResult;
use crate::bot::callbacks::AmountChoice;
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotDialogue, BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{ask_input, review_keyboard, review_score_keyboard, teacher_menu};

pub(super) async fn show_next(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        dialogue,
        ..
    } = screen;
    let services = &context.services;
    let item = services.reviews.take_next_from_queue(actor).await?;
    let Some(item) = item else {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, crate::bot::texts::REVIEW_QUEUE_EMPTY)
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    };

    show_item(screen, item).await
}

async fn show_item(screen: Screen<'_>, item: ReviewQueueItem) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        dialogue,
        ..
    } = screen;
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
        bot, message, deps, ..
    } = step;
    let Deps { dialogue, .. } = deps;

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
    let Deps { context, dialogue } = deps;
    let services = &context.services;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if !ensure_text_limit(bot, message.chat.id, text, USER_TEXT_MAX_LEN, "Комментарий").await?
    {
        return Ok(());
    }
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let result = services
        .reviews
        .review_submission(ReviewSubmission {
            submission_id,
            actor: screen.actor,
            decision,
            score,
            comment: text.to_owned(),
            reviewed_at: Utc::now(),
        })
        .await;

    match result {
        Ok(submission) => {
            reset_dialogue(dialogue).await?;
            let saved = submission.grade().map(|grade| {
                format!(
                    "\n🏅 Сохранено: {} − {} = {} из {}.",
                    grade.raw_score(),
                    grade.penalty(),
                    grade.final_score(),
                    grade.max_score(),
                )
            });
            bot.send_message(
                message.chat.id,
                format!(
                    "✅ Решение сохранено, студент получит уведомление.{}",
                    saved.unwrap_or_default()
                ),
            )
            .reply_markup(teacher_keyboard())
            .await?;
        }
        Err(error) if error.code() == ErrorCode::SubmissionAlreadyReviewed => {
            reset_dialogue(dialogue).await?;
            send_app_error(bot, message.chat.id, error).await?;
            show_home(step.chat(), telegram_user_id).await?;
        }
        Err(AppError::Domain(
            DomainError::ScoreRequired
            | DomainError::ScoreNotAllowed
            | DomainError::ScoreOutOfRange { .. },
        )) => {
            restart_review(screen, submission_id).await?;
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

async fn restart_review(screen: Screen<'_>, submission_id: SubmissionId) -> HandlerResult {
    match screen
        .context
        .services
        .reviews
        .find_for_review(screen.actor, submission_id)
        .await
    {
        Ok(item) => {
            screen
                .bot
                .send_message(screen.chat_id, texts::REVIEW_GRADING_CHANGED)
                .await?;
            show_item(screen, item).await
        }
        Err(error) => {
            if matches!(
                error.code(),
                ErrorCode::SubmissionAlreadyReviewed
                    | ErrorCode::NotFound
                    | ErrorCode::PermissionDenied
            ) {
                reset_dialogue(screen.dialogue).await?;
            }
            Err(error.into())
        }
    }
}

pub(super) async fn handle_decision(
    screen: Screen<'_>,
    callback_submission_id: SubmissionId,
    decision: ReviewDecision,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::AwaitingReviewDecision {
        submission_id,
        grading,
        penalty,
    }) = state
    else {
        return screen.stale().await;
    };
    if submission_id != callback_submission_id {
        return screen.stale().await;
    }
    match (decision, grading.max_score()) {
        (ReviewDecision::Accept, Some(max_score)) => {
            ask_review_score(
                screen.bot,
                screen.chat_id,
                submission_id,
                max_score,
                penalty,
                screen.dialogue,
            )
            .await
        }
        _ => {
            ask_review_comment(
                screen.bot,
                screen.chat_id,
                submission_id,
                decision,
                None,
                screen.dialogue,
            )
            .await
        }
    }
}

const SCORE_BUTTONS_LIMIT: u16 = 10;

async fn ask_review_score(
    bot: &Bot,
    chat_id: ChatId,
    submission_id: SubmissionId,
    max_score: u16,
    penalty: u16,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            penalty,
        }))
        .await?;
    if penalty > 0 {
        bot.send_message(
            chat_id,
            format!("⏳ Работа сдана с опозданием, штраф — {penalty}."),
        )
        .await?;
    }
    if max_score <= SCORE_BUTTONS_LIMIT {
        bot.send_message(chat_id, format!("Какой балл ставите из {max_score}?"))
            .reply_markup(review_score_keyboard(submission_id, max_score))
            .await?;
        return Ok(());
    }
    ask_input(
        bot,
        chat_id,
        &format!("Какой балл ставите? Введите число от 0 до {max_score}:"),
    )
    .await
}

async fn ask_review_comment(
    bot: &Bot,
    chat_id: ChatId,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    score: Option<u16>,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score,
        }))
        .await?;
    ask_input(bot, chat_id, texts::REVIEW_COMMENT_PROMPT).await
}

async fn confirm_score(
    bot: &Bot,
    chat_id: ChatId,
    score: u16,
    penalty: u16,
    max_score: u16,
) -> HandlerResult {
    if penalty > 0 {
        bot.send_message(
            chat_id,
            format!(
                "🏅 Итог: {score} − {penalty} = {} из {max_score}.",
                score.saturating_sub(penalty)
            ),
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn handle_review_score(
    screen: Screen<'_>,
    callback_submission_id: SubmissionId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::AwaitingReviewScore {
        submission_id,
        max_score,
        penalty,
    }) = state
    else {
        return screen.stale().await;
    };
    if submission_id != callback_submission_id {
        return screen.stale().await;
    }
    let score = match choice {
        AmountChoice::Preset(score) if score <= max_score => score,
        AmountChoice::Preset(_) | AmountChoice::Manual => {
            return screen
                .ask(&format!("Введите целое число от 0 до {max_score}:"))
                .await;
        }
    };
    confirm_score(screen.bot, screen.chat_id, score, penalty, max_score).await?;
    ask_review_comment(
        screen.bot,
        screen.chat_id,
        submission_id,
        ReviewDecision::Accept,
        Some(score),
        screen.dialogue,
    )
    .await
}

fn parse_score(text: &str, max_score: u16) -> Option<u16> {
    let score = parse_number(text)?;
    (score <= max_score).then_some(score)
}
