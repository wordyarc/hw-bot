use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardMarkup, MessageId};

use super::home::show_home;
use super::input::{parse_number, require_text, require_text_within};
use super::reset_dialogue;
use super::teacher::show_student_grades_page;
use crate::app::{AppError, ErrorCode, ReviewQueueItem, ReviewSubmission};
use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::callbacks::AmountChoice;
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::{log_handler_error, optional_stat, send_app_error};
use crate::bot::state::{BotState, FlowState, GrantOrigin, ReviewTarget, ScreenState};
use crate::bot::texts;
use crate::bot::views::{
    SCORE_BUTTONS_LIMIT, makeup_followup_keyboard, review_decision_keyboard, review_hub_keyboard,
    review_keyboard, review_score_keyboard, send_message_chunks, teacher_keyboard, teacher_menu,
};
use crate::domain::{
    DomainError, Grade, Grading, LatePenalty, MaxScore, ReviewDecision, ReviewScore, SubmissionId,
    SubmissionKind, SubmissionStatus, TeachingCourseId, TelegramUserId, USER_TEXT_MAX_LEN, UserId,
};
use crate::pagination::PageRequest;

pub(super) async fn show_hub(screen: Screen<'_>) -> HandlerResult {
    let reviews = &screen.services().reviews;
    let makeups = reviews.makeup_size(screen.actor()).await?;
    if makeups == 0 {
        return show_next(screen, Queue::Main).await;
    }
    let queue = reviews.queue_size(screen.actor()).await?;
    reset_dialogue(screen.dialogue()).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::review_hub(queue, makeups))
        .reply_markup(review_hub_keyboard(queue, makeups))
        .await?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum Queue {
    Main,
    Makeup,
}

pub(super) async fn show_next(screen: Screen<'_>, queue: Queue) -> HandlerResult {
    let actor = screen.actor();
    let reviews = &screen.services().reviews;
    let next = match queue {
        Queue::Main => reviews.take_next_from_queue(actor).await?,
        Queue::Makeup => reviews.take_next_makeup(actor).await?,
    };
    if let Some(item) = next {
        return show_item(screen, item).await;
    }

    let (empty, waiting, menu) = match queue {
        Queue::Main => {
            let makeups = reviews.makeup_size(actor).await?;
            (
                texts::REVIEW_QUEUE_EMPTY,
                (makeups > 0).then(|| texts::makeup_queue_line(makeups)),
                teacher_menu((makeups == 0).then_some(0)),
            )
        }
        Queue::Makeup => {
            let queue = reviews.queue_size(actor).await?;
            (
                texts::MAKEUP_QUEUE_EMPTY,
                (queue > 0).then(|| texts::review_queue_line(queue)),
                teacher_menu(Some(queue)),
            )
        }
    };
    reset_dialogue(screen.dialogue()).await?;
    let text = waiting.map_or_else(|| empty.to_owned(), |line| format!("{empty}\n{line}"));
    screen
        .bot()
        .send_message(screen.chat_id(), text)
        .reply_markup(menu)
        .await?;
    Ok(())
}

pub(super) async fn open(screen: Screen<'_>, submission_id: SubmissionId) -> HandlerResult {
    let item = screen
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id)
        .await?;

    show_item(screen, item).await
}

fn review_target(item: &ReviewQueueItem) -> ReviewTarget {
    match item.submission.kind() {
        SubmissionKind::Regular => ReviewTarget::Regular {
            grading: item.assignment.grading(),
            late: item
                .assignment
                .late_penalty_at(item.submission.submitted_at()),
        },
        SubmissionKind::Makeup => ReviewTarget::Makeup,
    }
}

fn accept_label(item: &ReviewQueueItem) -> Option<String> {
    match item.submission.kind() {
        SubmissionKind::Regular => Some(texts::BUTTON_ACCEPT.to_owned()),
        SubmissionKind::Makeup => item
            .assignment
            .makeup_grade()
            .ok()
            .map(|grade| match grade {
                Some(grade) => texts::accept_makeup_button(grade.raw_score()),
                None => texts::BUTTON_ACCEPT_MAKEUP.to_owned(),
            }),
    }
}

fn item_keyboard(item: &ReviewQueueItem) -> InlineKeyboardMarkup {
    review_keyboard(
        item.submission.id(),
        item.assignment.teaching_course_id(),
        item.student.id(),
        accept_label(item),
    )
}

async fn show_item(screen: Screen<'_>, item: ReviewQueueItem) -> HandlerResult {
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let target = review_target(&item);
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id: item.submission.id(),
            target,
        }))
        .await?;
    bot.send_message(chat_id, review_card_summary(&item, target))
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
    let submission_header = match send_previous_attempts(bot, chat_id, &item).await {
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
        Some(item_keyboard(&item)),
    )
    .await
}

fn review_card_summary(item: &ReviewQueueItem, target: ReviewTarget) -> String {
    let mut lines = vec![texts::review_summary(
        item.subject.name().as_str(),
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
        &item.group_code,
        item.submission.attempt_number(),
    )];
    match target {
        ReviewTarget::Regular {
            grading: Grading::Score { max_score },
            late: Some(late),
        } => lines.push(texts::late_submission_line(late, max_score.get())),
        ReviewTarget::Regular { .. } => {}
        ReviewTarget::Makeup => {
            lines.push(texts::makeup_review_line(
                item.assignment.makeup(),
                item.assignment.grading(),
            ));
            if let Some(pool) = item.makeup {
                lines.push(texts::review_makeup_attempts(
                    pool.balance.remaining(),
                    pool.balance.allowed(),
                ));
            }
        }
    }
    lines.join("\n")
}

async fn send_previous_attempts(
    bot: &Bot,
    chat_id: ChatId,
    item: &ReviewQueueItem,
) -> HandlerResult {
    if item.earlier_attempts_hidden {
        bot.send_message(
            chat_id,
            texts::earlier_attempts_hidden(item.previous_attempts.len()),
        )
        .await?;
    }
    for attempt in &item.previous_attempts {
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
    max_score: MaxScore,
    late: Option<LatePenalty>,
) -> HandlerResult {
    let max_score = max_score.get();
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(score) = parse_score(&text, max_score) else {
        bot.send_message(chat_id, texts::score_prompt(max_score))
            .await?;
        return Ok(());
    };
    ask_review_comment(
        step.chat(),
        submission_id,
        ReviewDecision::Accept,
        SubmissionKind::Regular,
        Some(Grade::new(score, max_score, late)),
    )
    .await
}

pub(super) async fn submit_comment(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    score: Option<ReviewScore>,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let telegram_user_id = step.telegram_user_id();
    let dialogue = step.dialogue();
    let services = step.services();

    let Some(text) =
        require_text_within(bot, message, USER_TEXT_MAX_LEN, texts::FIELD_COMMENT).await?
    else {
        return Ok(());
    };
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
            comment: text.into_owned(),
            reviewed_at: Utc::now(),
        })
        .await;

    match result {
        Ok(reviewed) => {
            reset_dialogue(dialogue).await?;
            let submission = &reviewed.submission;
            let saved = texts::grade_line(submission.kind(), submission.grade())
                .map(|line| format!("\n{line}"))
                .unwrap_or_default();
            bot.send_message(chat_id, format!("{}{saved}", texts::REVIEW_SAVED))
                .reply_markup(teacher_keyboard())
                .await?;
            if submission.kind() == SubmissionKind::Makeup {
                send_makeup_followup(
                    screen,
                    MakeupFollowup {
                        course_id: reviewed.course_id,
                        student_id: submission.student_id(),
                        offer_grant: submission.status() == SubmissionStatus::Rejected,
                        sandbox: reviewed.sandbox,
                    },
                )
                .await?;
            }
        }
        Err(error) if review_closed(&error) => {
            close_review(screen, telegram_user_id, error).await?;
        }
        Err(AppError::Domain(
            DomainError::ScoreRequired
            | DomainError::ScoreNotAllowed
            | DomainError::ScoreScaleMismatch { .. },
        )) => {
            restart_review(
                screen,
                telegram_user_id,
                submission_id,
                texts::REVIEW_GRADING_CHANGED,
            )
            .await?;
        }
        Err(AppError::Domain(DomainError::MakeupNotOffered)) => {
            restart_review(
                screen,
                telegram_user_id,
                submission_id,
                texts::REVIEW_MAKEUP_WITHDRAWN,
            )
            .await?;
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

struct MakeupFollowup {
    course_id: TeachingCourseId,
    student_id: UserId,
    offer_grant: bool,
    sandbox: bool,
}

async fn send_makeup_followup(screen: Screen<'_>, followup: MakeupFollowup) -> HandlerResult {
    let services = screen.services();
    let makeups = if followup.sandbox {
        None
    } else {
        optional_stat(
            services.reviews.makeup_size(screen.actor()).await,
            "makeups_left",
        )
    };
    let pool = if followup.offer_grant {
        optional_stat(
            services
                .courses
                .student_makeup(screen.actor(), followup.course_id, followup.student_id)
                .await,
            "student_makeup",
        )
        .flatten()
    } else {
        None
    };
    let mut lines: Vec<String> = makeups.map(texts::makeups_left).into_iter().collect();
    lines.extend(pool.map(|pool| {
        texts::review_makeup_attempts(pool.balance.remaining(), pool.balance.allowed())
    }));
    if lines.is_empty() {
        return Ok(());
    }
    let text = lines.join("\n");
    let next = makeups.is_some_and(|makeups| makeups > 0);
    if !next && pool.is_none() {
        screen.bot().send_message(screen.chat_id(), text).await?;
        return Ok(());
    }
    let sent = screen
        .bot()
        .send_message(screen.chat_id(), text)
        .reply_markup(makeup_followup_keyboard(next, pool.is_some()))
        .await?;
    if pool.is_some() {
        screen
            .dialogue()
            .update(BotState::Screen(ScreenState::MakeupReviewed {
                message_id: sent.id,
                course_id: followup.course_id,
                student_id: followup.student_id,
                sandbox: followup.sandbox,
            }))
            .await?;
    }
    Ok(())
}

pub(super) async fn grant_makeup_attempt(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(grant) = state.makeup_grant(message_id) else {
        return screen.stale().await;
    };
    let notice = match screen
        .services()
        .courses
        .grant_makeup_attempt(screen.actor(), grant.course_id, grant.student_id)
        .await
    {
        Ok(_) => texts::MAKEUP_ATTEMPT_GRANTED.to_owned(),
        Err(AppError::Domain(DomainError::MakeupNotOffered)) => {
            texts::MAKEUP_GRANT_UNAVAILABLE.to_owned()
        }
        Err(error) => return Err(error.into()),
    };
    reset_dialogue(screen.dialogue()).await?;
    screen.bot().send_message(screen.chat_id(), notice).await?;
    match grant.origin {
        GrantOrigin::Review { sandbox } => {
            send_makeup_followup(
                screen,
                MakeupFollowup {
                    course_id: grant.course_id,
                    student_id: grant.student_id,
                    offer_grant: true,
                    sandbox,
                },
            )
            .await
        }
        GrantOrigin::Grades { list } => {
            show_student_grades_page(
                screen,
                grant.course_id,
                grant.student_id,
                PageRequest::default(),
                list,
            )
            .await
        }
    }
}

fn review_closed(error: &AppError) -> bool {
    matches!(
        error.code(),
        ErrorCode::SubmissionAlreadyReviewed | ErrorCode::NotFound | ErrorCode::PermissionDenied
    )
}

async fn close_review(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    error: AppError,
) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    send_app_error(screen.bot(), screen.chat_id(), error).await?;
    show_home(screen.chat(), telegram_user_id).await
}

async fn reload_item(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    submission_id: SubmissionId,
) -> HandlerResult<Option<ReviewQueueItem>> {
    match screen
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id)
        .await
    {
        Ok(item) => Ok(Some(item)),
        Err(error) if review_closed(&error) => {
            close_review(screen, telegram_user_id, error).await?;
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

async fn restart_review(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    submission_id: SubmissionId,
    reason: &str,
) -> HandlerResult {
    match reload_item(screen, telegram_user_id, submission_id).await? {
        Some(item) => restart_with(screen, item, reason).await,
        None => Ok(()),
    }
}

pub(super) async fn handle_decision(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    callback_submission_id: SubmissionId,
    decision: ReviewDecision,
    state: BotState,
) -> HandlerResult {
    let Some(target) = state.review_decision_target(callback_submission_id) else {
        return screen.stale().await;
    };
    let submission_id = callback_submission_id;
    let chat = screen.chat();
    match (decision, target) {
        (
            ReviewDecision::Accept,
            ReviewTarget::Regular {
                grading: Grading::Score { max_score },
                late,
            },
        ) => ask_review_score(chat, submission_id, max_score, late).await,
        (_, ReviewTarget::Regular { .. }) => {
            ask_review_comment(chat, submission_id, decision, SubmissionKind::Regular, None).await
        }
        (ReviewDecision::Accept, ReviewTarget::Makeup) => {
            let Some(item) = reload_item(screen, telegram_user_id, submission_id).await? else {
                return Ok(());
            };
            let Ok(grade) = item.assignment.makeup_grade() else {
                return restart_with(screen, item, texts::REVIEW_MAKEUP_WITHDRAWN).await;
            };
            ask_review_comment(chat, submission_id, decision, SubmissionKind::Makeup, grade).await
        }
        (ReviewDecision::Reject, ReviewTarget::Makeup) => {
            ask_review_comment(chat, submission_id, decision, SubmissionKind::Makeup, None).await
        }
    }
}

async fn restart_with(screen: Screen<'_>, item: ReviewQueueItem, reason: &str) -> HandlerResult {
    screen
        .bot()
        .send_message(screen.chat_id(), reason)
        .reply_markup(teacher_keyboard())
        .await?;
    show_item(screen, item).await
}

async fn ask_review_score(
    chat: Chat<'_>,
    submission_id: SubmissionId,
    scale: MaxScore,
    late: Option<LatePenalty>,
) -> HandlerResult {
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score: scale,
            late,
        }))
        .await?;
    let max_score = scale.get();
    let question = texts::review_score_question(max_score, late);
    if max_score <= SCORE_BUTTONS_LIMIT {
        chat.bot()
            .send_message(chat.chat_id(), question)
            .reply_markup(review_score_keyboard(submission_id, max_score))
            .await?;
        return Ok(());
    }
    chat.ask(&format!("{question} {}", texts::score_prompt(max_score)))
        .await
}

async fn ask_review_comment(
    chat: Chat<'_>,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    kind: SubmissionKind,
    grade: Option<Grade>,
) -> HandlerResult {
    let score = match kind {
        SubmissionKind::Regular => grade.map(|grade| ReviewScore {
            value: grade.raw_score(),
            max_score: grade.max_score(),
        }),
        SubmissionKind::Makeup => None,
    };
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score,
        }))
        .await?;
    chat.bot()
        .send_message(
            chat.chat_id(),
            texts::review_decision(decision, kind, grade),
        )
        .reply_markup(review_decision_keyboard(submission_id))
        .await?;
    chat.ask(texts::REVIEW_COMMENT_PROMPT).await
}

pub(super) async fn change_decision(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    submission_id: SubmissionId,
    state: &BotState,
) -> HandlerResult {
    if !state.is_awaiting_review_comment(submission_id) {
        return screen.stale().await;
    }
    let Some(item) = reload_item(screen, telegram_user_id, submission_id).await? else {
        return Ok(());
    };
    let target = review_target(&item);
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id,
            target,
        }))
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::REVIEW_DECISION_RESET)
        .reply_markup(teacher_keyboard())
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), review_card_summary(&item, target))
        .reply_markup(item_keyboard(&item))
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
    let max_score = max_score.get();
    let score = match choice {
        AmountChoice::Preset(score) if score <= max_score => score,
        AmountChoice::Preset(_) | AmountChoice::Manual => {
            return screen.ask(&texts::score_prompt(max_score)).await;
        }
    };
    ask_review_comment(
        screen.chat(),
        submission_id,
        ReviewDecision::Accept,
        SubmissionKind::Regular,
        Some(Grade::new(score, max_score, late)),
    )
    .await
}

fn parse_score(text: &str, max_score: u16) -> Option<u16> {
    let score = parse_number(text)?;
    (score <= max_score).then_some(score)
}
