use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::ChatId;

use super::home::show_home;
use super::input::{ensure_text_limit, parse_number, require_text};
use super::reset_dialogue;
use crate::app::{AppError, ErrorCode, ReviewQueueItem, ReviewSubmission};
use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::callbacks::AmountChoice;
use crate::bot::context::{Screen, TextStep};
use crate::bot::errors::{log_handler_error, send_app_error};
use crate::bot::state::{BotDialogue, BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{
    ask_input, review_decision_keyboard, review_keyboard, review_score_keyboard,
    send_message_chunks, teacher_keyboard, teacher_menu,
};
use crate::domain::{
    DomainError, Grade, LatePenalty, ReviewDecision, Submission, SubmissionId, TelegramUserId,
    USER_TEXT_MAX_LEN,
};

pub(super) async fn show_next(screen: Screen<'_>) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let item = services.reviews.take_next_from_queue(actor).await?;
    let Some(item) = item else {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, texts::REVIEW_QUEUE_EMPTY)
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    };

    show_item(screen, item).await
}

pub(super) async fn open(screen: Screen<'_>, submission_id: SubmissionId) -> HandlerResult {
    let item = screen
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id)
        .await?;

    show_item(screen, item).await
}

async fn show_item(screen: Screen<'_>, item: ReviewQueueItem) -> HandlerResult {
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let late = item
        .assignment
        .late_penalty_at(item.submission.submitted_at());
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id: item.submission.id(),
            grading: item.assignment.grading(),
            late,
        }))
        .await?;
    let keyboard = review_keyboard(
        item.submission.id(),
        item.assignment.teaching_course_id(),
        item.student.id(),
    );
    bot.send_message(chat_id, review_card_summary(&item, late))
        .await?;
    bot.send_message(chat_id, texts::ASSIGNMENT_BODY_HEADER)
        .await?;
    send_message_chunks(
        bot,
        chat_id,
        item.assignment.body().as_str().to_owned(),
        None,
    )
    .await?;
    let submission_header =
        match send_previous_attempts(bot, chat_id, &item.previous_attempts).await {
            Ok(()) => texts::SUBMISSION_HEADER.to_owned(),
            Err(error) if error.is_invalid_token() => return Err(error),
            Err(error) => {
                log_handler_error(&error);
                format!(
                    "{}\n\n{}",
                    texts::PREVIOUS_ATTEMPTS_INCOMPLETE,
                    texts::SUBMISSION_HEADER
                )
            }
        };
    bot.send_message(chat_id, submission_header).await?;
    send_message_chunks(
        bot,
        chat_id,
        item.submission.content().as_str().to_owned(),
        Some(keyboard),
    )
    .await
}

fn review_card_summary(item: &ReviewQueueItem, late: Option<LatePenalty>) -> String {
    let mut summary = texts::review_summary(
        item.subject.name().as_str(),
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
        item.submission.attempt_number(),
    );
    if let (Some(late), Some(max_score)) = (late, item.assignment.grading().max_score()) {
        summary.push('\n');
        summary.push_str(&texts::late_submission_line(late, max_score));
    }
    summary
}

async fn send_previous_attempts(
    bot: &Bot,
    chat_id: ChatId,
    attempts: &[Submission],
) -> HandlerResult {
    for attempt in attempts {
        let text = texts::previous_attempt(
            attempt.attempt_number(),
            texts::submission_status(attempt.status()),
            attempt.content().as_str(),
            attempt.review_comment().map(|comment| comment.as_str()),
        );
        send_message_chunks(bot, chat_id, text, None).await?;
    }
    Ok(())
}

pub(super) async fn submit_score_text(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    max_score: u16,
    late: Option<LatePenalty>,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let dialogue = step.dialogue();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(score) = parse_score(text, max_score) else {
        bot.send_message(chat_id, texts::score_error(max_score))
            .await?;
        return Ok(());
    };
    ask_review_comment(
        bot,
        chat_id,
        submission_id,
        ReviewDecision::Accept,
        Some(Grade::new(score, max_score, late)),
        dialogue,
    )
    .await
}

pub(super) async fn submit_comment(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    score: Option<u16>,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let telegram_user_id = step.telegram_user_id();
    let dialogue = step.dialogue();
    let services = step.services();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if !ensure_text_limit(bot, chat_id, text, USER_TEXT_MAX_LEN, texts::FIELD_COMMENT).await? {
        return Ok(());
    }
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let result = services
        .reviews
        .review_submission(ReviewSubmission {
            submission_id,
            actor: screen.actor(),
            decision,
            score,
            comment: text.to_owned(),
            reviewed_at: Utc::now(),
        })
        .await;

    match result {
        Ok(submission) => {
            reset_dialogue(dialogue).await?;
            let saved = submission
                .grade()
                .map(|grade| format!("\n{}", texts::grade_line(grade)))
                .unwrap_or_default();
            bot.send_message(chat_id, format!("{}{saved}", texts::REVIEW_SAVED))
                .reply_markup(teacher_keyboard())
                .await?;
        }
        Err(error) if error.code() == ErrorCode::SubmissionAlreadyReviewed => {
            reset_dialogue(dialogue).await?;
            send_app_error(bot, chat_id, error).await?;
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
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id)
        .await
    {
        Ok(item) => {
            screen
                .bot()
                .send_message(screen.chat_id(), texts::REVIEW_GRADING_CHANGED)
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
                reset_dialogue(screen.dialogue()).await?;
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
        late,
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
                screen.bot(),
                screen.chat_id(),
                submission_id,
                max_score,
                late,
                screen.dialogue(),
            )
            .await
        }
        _ => {
            ask_review_comment(
                screen.bot(),
                screen.chat_id(),
                submission_id,
                decision,
                None,
                screen.dialogue(),
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
    late: Option<LatePenalty>,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            late,
        }))
        .await?;
    let question = texts::review_score_question(max_score, late);
    if max_score <= SCORE_BUTTONS_LIMIT {
        bot.send_message(chat_id, question)
            .reply_markup(review_score_keyboard(submission_id, max_score))
            .await?;
        return Ok(());
    }
    ask_input(
        bot,
        chat_id,
        &format!("{question} {}", texts::score_prompt(max_score)),
    )
    .await
}

async fn ask_review_comment(
    bot: &Bot,
    chat_id: ChatId,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    grade: Option<Grade>,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score: grade.map(Grade::raw_score),
        }))
        .await?;
    bot.send_message(chat_id, texts::review_decision(decision, grade))
        .reply_markup(review_decision_keyboard(submission_id))
        .await?;
    ask_input(bot, chat_id, texts::REVIEW_COMMENT_PROMPT).await
}

pub(super) async fn change_decision(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    callback_submission_id: SubmissionId,
    state: &BotState,
) -> HandlerResult {
    let Some(submission_id) = state.awaiting_review_comment(callback_submission_id) else {
        return screen.stale().await;
    };
    let item = match screen
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id)
        .await
    {
        Ok(item) => item,
        Err(error)
            if matches!(
                error.code(),
                ErrorCode::SubmissionAlreadyReviewed
                    | ErrorCode::NotFound
                    | ErrorCode::PermissionDenied
            ) =>
        {
            reset_dialogue(screen.dialogue()).await?;
            send_app_error(screen.bot(), screen.chat_id(), error).await?;
            return show_home(screen.chat(), telegram_user_id).await;
        }
        Err(error) => return Err(error.into()),
    };
    let late = item
        .assignment
        .late_penalty_at(item.submission.submitted_at());
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id,
            grading: item.assignment.grading(),
            late,
        }))
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::REVIEW_DECISION_RESET)
        .reply_markup(teacher_keyboard())
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), review_card_summary(&item, late))
        .reply_markup(review_keyboard(
            item.submission.id(),
            item.assignment.teaching_course_id(),
            item.student.id(),
        ))
        .await?;
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
        late,
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
            return screen.ask(&texts::score_prompt(max_score)).await;
        }
    };
    ask_review_comment(
        screen.bot(),
        screen.chat_id(),
        submission_id,
        ReviewDecision::Accept,
        Some(Grade::new(score, max_score, late)),
        screen.dialogue(),
    )
    .await
}

fn parse_score(text: &str, max_score: u16) -> Option<u16> {
    let score = parse_number(text)?;
    (score <= max_score).then_some(score)
}
