use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, InlineKeyboardMarkup, MessageId};

use super::home::show_home;
use super::input::{parse_number, require_text, require_text_within};
use super::reset_dialogue;
use super::teacher::send_student_grades;
use crate::app::{AppError, ErrorCode, ReviewQueueItem, ReviewSubmission, ReviewedSubmission};
use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::callbacks::AmountChoice;
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::{log_handler_error, optional_stat, send_app_error};
use crate::bot::state::{
    BotState, FlowState, GrantOrigin, ReviewOrigin, ReviewTarget, ScreenState,
};
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{
    SCORE_BUTTONS_LIMIT, list_followup_keyboard, list_text, makeup_followup_keyboard,
    queue_followup_keyboard, review_decision_keyboard, review_hub_keyboard, review_keyboard,
    review_list_keyboard, review_score_keyboard, send_message_chunks, teacher_keyboard,
    teacher_menu,
};
use crate::domain::{
    DomainError, Grade, Grading, LatePenalty, MaxScore, ReviewDecision, ReviewScore, SubjectId,
    SubmissionId, SubmissionKind, SubmissionStatus, TeachingCourseId, TelegramUserId,
    USER_TEXT_MAX_LEN, UserId,
};
use crate::pagination::PageRequest;

pub(super) async fn show_hub(screen: Screen<'_>) -> HandlerResult {
    let reviews = &screen.services().reviews;
    let subjects = reviews.queue_by_subject(screen.actor()).await?;
    let makeups = reviews.makeup_size(screen.actor()).await?;
    let queue: u32 = subjects.iter().map(|subject| subject.pending).sum();
    reset_dialogue(screen.dialogue()).await?;
    if queue == 0 && makeups == 0 {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::REVIEW_QUEUE_EMPTY)
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    }
    screen
        .bot()
        .send_message(screen.chat_id(), texts::review_hub(queue, makeups))
        .reply_markup(review_hub_keyboard(&subjects, makeups))
        .await?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) enum Queue {
    Main(Option<SubjectId>),
    Makeup,
}

pub(super) async fn show_next(screen: Screen<'_>, queue: Queue) -> HandlerResult {
    let actor = screen.actor();
    let reviews = &screen.services().reviews;
    let (next, origin) = match queue {
        Queue::Main(subject) => (
            reviews
                .take_next_from_queue(actor, subject, screen.now())
                .await?,
            ReviewOrigin::Queue { subject },
        ),
        Queue::Makeup => (
            reviews.take_next_makeup(actor, screen.now()).await?,
            ReviewOrigin::Makeups,
        ),
    };
    if let Some(item) = next {
        return show_item(screen, item, origin).await;
    }

    let (empty, waiting, menu) = match queue {
        Queue::Main(None) => {
            let makeups = reviews.makeup_size(actor).await?;
            (
                texts::REVIEW_QUEUE_EMPTY.to_owned(),
                (makeups > 0).then(|| texts::makeup_queue_line(makeups)),
                teacher_menu((makeups == 0).then_some(0)),
            )
        }
        Queue::Main(Some(subject)) => {
            let subject = reviews.subject(actor, subject).await?;
            let queue = reviews.queue_size(actor, None).await?;
            (
                texts::subject_queue_empty(subject.name().as_str()),
                (queue > 0).then(|| texts::review_queue_line(queue)),
                teacher_menu(Some(queue)),
            )
        }
        Queue::Makeup => {
            let queue = reviews.queue_size(actor, None).await?;
            (
                texts::MAKEUP_QUEUE_EMPTY.to_owned(),
                (queue > 0).then(|| texts::review_queue_line(queue)),
                teacher_menu(Some(queue)),
            )
        }
    };
    reset_dialogue(screen.dialogue()).await?;
    let text = match waiting {
        Some(line) => format!("{empty}\n{line}"),
        None => empty,
    };
    screen
        .bot()
        .send_message(screen.chat_id(), text)
        .reply_markup(menu)
        .await?;
    Ok(())
}

pub(super) async fn show_list(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let reviews = &screen.services().reviews;
    let timezone = screen.timezone();
    let submissions = reviews.queue_page(actor, page).await?;
    reset_dialogue(screen.dialogue()).await?;
    if submissions.items.is_empty() && page.page == 0 {
        bot.send_message(chat_id, texts::REVIEW_QUEUE_EMPTY)
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    }

    let total = optional_stat(reviews.queue_size(actor, None).await, "queue_total");
    let entries = submissions.items.iter().map(|submission| {
        texts::review_list_entry(
            &submission.student_name,
            &submission.assignment_title,
            &submission.subject_name,
            &timezone.format_short(submission.submitted_at),
        )
    });
    let text = list_text(&texts::review_list_header(total), entries);
    send_message_chunks(bot, chat_id, &text, None).await?;
    bot.send_message(chat_id, texts::REVIEW_LIST_CHOOSE)
        .reply_markup(review_list_keyboard(
            &submissions.items,
            submissions.page,
            submissions.next_page,
        ))
        .await?;
    Ok(())
}

pub(super) async fn open(
    screen: Screen<'_>,
    submission_id: SubmissionId,
    origin: ReviewOrigin,
) -> HandlerResult {
    let item = screen
        .services()
        .reviews
        .find_for_review(screen.actor(), submission_id, screen.now())
        .await?;

    show_item(screen, item, origin).await
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

fn offers_rework(item: &ReviewQueueItem, now: DateTime<Utc>) -> bool {
    item.submission.kind() == SubmissionKind::Regular && item.assignment.is_open_for_regular_at(now)
}

fn item_keyboard(item: &ReviewQueueItem, now: DateTime<Utc>) -> InlineKeyboardMarkup {
    review_keyboard(
        item.submission.id(),
        item.assignment.teaching_course_id(),
        item.student.id(),
        accept_label(item),
        offers_rework(item, now),
    )
}

async fn show_item(
    screen: Screen<'_>,
    item: ReviewQueueItem,
    origin: ReviewOrigin,
) -> HandlerResult {
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let target = review_target(&item);
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id: item.submission.id(),
            target,
            origin,
        }))
        .await?;
    bot.send_message(
        chat_id,
        review_card_summary(&item, target, screen.timezone()),
    )
    .await?;
    bot.send_message(chat_id, texts::ASSIGNMENT_BODY_HEADER)
        .await?;
    send_message_chunks(bot, chat_id, item.assignment.body().as_str(), None).await?;
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
        item.submission.content().as_str(),
        Some(item_keyboard(&item, screen.now())),
    )
    .await
}

fn attempt_line(item: &ReviewQueueItem) -> String {
    match item.submission.kind() {
        SubmissionKind::Regular => texts::regular_attempt(
            item.history.used_attempts() + 1,
            item.history.allowed_attempts_for(&item.assignment),
        ),
        SubmissionKind::Makeup => texts::makeup_attempt(item.submission.attempt_number()),
    }
}

fn review_card_summary(
    item: &ReviewQueueItem,
    target: ReviewTarget,
    timezone: &BotTimezone,
) -> String {
    let mut lines = vec![texts::review_summary(
        item.subject.name().as_str(),
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
        &item.group_code,
        &attempt_line(item),
        &timezone.format_moment(item.submission.submitted_at()),
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
        send_message_chunks(bot, chat_id, &text, None).await?;
    }
    Ok(())
}

pub(super) async fn submit_score_text(
    step: TextStep<'_>,
    submission_id: SubmissionId,
    max_score: MaxScore,
    late: Option<LatePenalty>,
    origin: ReviewOrigin,
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
    score_chosen(
        step.chat(),
        submission_id,
        Grade::new(score, max_score, late),
        origin,
    )
    .await
}

pub(super) struct PendingComment {
    pub submission_id: SubmissionId,
    pub decision: ReviewDecision,
    pub score: Option<ReviewScore>,
    pub return_attempt: bool,
    pub origin: ReviewOrigin,
}

pub(super) async fn submit_comment(step: TextStep<'_>, pending: PendingComment) -> HandlerResult {
    let PendingComment {
        submission_id,
        decision,
        score,
        return_attempt,
        origin,
    } = pending;
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
            reviewed_at: step.now(),
            return_attempt,
        })
        .await;

    match result {
        Ok(reviewed) => {
            reset_dialogue(dialogue).await?;
            bot.send_message(chat_id, saved_text(&reviewed))
                .reply_markup(teacher_keyboard())
                .await?;
            let submission = &reviewed.submission;
            match submission.kind() {
                SubmissionKind::Makeup => {
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
                SubmissionKind::Regular => {
                    send_queue_followup(screen, origin, reviewed.subject.name().as_str()).await?;
                }
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
                origin,
            )
            .await?;
        }
        Err(AppError::Domain(DomainError::MakeupNotOffered)) => {
            restart_review(
                screen,
                telegram_user_id,
                submission_id,
                texts::REVIEW_MAKEUP_WITHDRAWN,
                origin,
            )
            .await?;
        }
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn saved_text(reviewed: &ReviewedSubmission) -> String {
    let submission = &reviewed.submission;
    let mut lines = vec![texts::REVIEW_SAVED.to_owned()];
    lines.extend(texts::grade_line(submission.kind(), submission.grade()));
    lines.extend(
        reviewed
            .attempts
            .map(|attempts| texts::attempt_returned(attempts.remaining, attempts.allowed)),
    );
    lines.join("\n")
}

async fn send_queue_followup(
    screen: Screen<'_>,
    origin: ReviewOrigin,
    subject_name: &str,
) -> HandlerResult {
    let reviews = &screen.services().reviews;
    let actor = screen.actor();
    let (text, keyboard) = match origin {
        ReviewOrigin::Queue {
            subject: Some(subject),
        } => {
            let left = optional_stat(
                reviews.queue_size(actor, Some(subject)).await,
                "subject_queue_left",
            );
            if left == Some(0) {
                let rest = optional_stat(reviews.queue_size(actor, None).await, "queue_left");
                (
                    texts::subject_done(subject_name, rest),
                    Some(queue_followup_keyboard(Some(subject), false)),
                )
            } else {
                (
                    texts::subject_left(subject_name, left),
                    Some(queue_followup_keyboard(Some(subject), true)),
                )
            }
        }
        ReviewOrigin::Queue { subject: None } => {
            let left = optional_stat(reviews.queue_size(actor, None).await, "queue_left");
            (
                texts::queue_left(left),
                Some(queue_followup_keyboard(None, left != Some(0))),
            )
        }
        ReviewOrigin::List => {
            let left = optional_stat(reviews.queue_size(actor, None).await, "queue_left");
            (
                texts::queue_left(left),
                (left != Some(0)).then(list_followup_keyboard),
            )
        }
        ReviewOrigin::Makeups | ReviewOrigin::Sandbox => return Ok(()),
    };
    let message = screen.bot().send_message(screen.chat_id(), text);
    match keyboard {
        Some(keyboard) => message.reply_markup(keyboard).await?,
        None => message.await?,
    };
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
                .student_makeup(
                    screen.actor(),
                    followup.course_id,
                    followup.student_id,
                    screen.now(),
                )
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
        .grant_makeup_attempt(
            screen.actor(),
            grant.course_id,
            grant.student_id,
            screen.now(),
        )
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
            let card = optional_stat(
                screen
                    .services()
                    .courses
                    .student_grades_card(
                        screen.actor(),
                        grant.course_id,
                        grant.student_id,
                        PageRequest::default(),
                        screen.now(),
                    )
                    .await,
                "student_grades",
            );
            match card {
                Some(card) => send_student_grades(screen, grant.course_id, card, list).await,
                None => Ok(()),
            }
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
        .find_for_review(screen.actor(), submission_id, screen.now())
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
    origin: ReviewOrigin,
) -> HandlerResult {
    match reload_item(screen, telegram_user_id, submission_id).await? {
        Some(item) => restart_with(screen, item, reason, origin).await,
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
    let Some((target, origin)) = state.review_decision_target(callback_submission_id) else {
        return screen.stale().await;
    };
    let submission_id = callback_submission_id;
    let chat = screen.chat();
    let comment = |kind, grade| CommentStep {
        submission_id,
        decision,
        kind,
        grade,
        return_attempt: false,
        origin,
    };
    match (decision, target) {
        (
            ReviewDecision::Accept,
            ReviewTarget::Regular {
                grading: Grading::Score { max_score },
                late,
            },
        ) => ask_review_score(chat, submission_id, max_score, late, origin).await,
        (_, ReviewTarget::Regular { .. }) => {
            ask_review_comment(chat, comment(SubmissionKind::Regular, None)).await
        }
        (ReviewDecision::Accept, ReviewTarget::Makeup) => {
            let Some(item) = reload_item(screen, telegram_user_id, submission_id).await? else {
                return Ok(());
            };
            let Ok(grade) = item.assignment.makeup_grade() else {
                return restart_with(screen, item, texts::REVIEW_MAKEUP_WITHDRAWN, origin).await;
            };
            ask_review_comment(chat, comment(SubmissionKind::Makeup, grade)).await
        }
        (ReviewDecision::Reject, ReviewTarget::Makeup) => {
            ask_review_comment(chat, comment(SubmissionKind::Makeup, None)).await
        }
    }
}

pub(super) async fn handle_rework(
    screen: Screen<'_>,
    submission_id: SubmissionId,
    state: &BotState,
) -> HandlerResult {
    let Some(origin) = state.rework_origin(submission_id) else {
        return screen.stale().await;
    };
    ask_review_comment(
        screen.chat(),
        CommentStep {
            submission_id,
            decision: ReviewDecision::Reject,
            kind: SubmissionKind::Regular,
            grade: None,
            return_attempt: true,
            origin,
        },
    )
    .await
}

async fn restart_with(
    screen: Screen<'_>,
    item: ReviewQueueItem,
    reason: &str,
    origin: ReviewOrigin,
) -> HandlerResult {
    screen
        .bot()
        .send_message(screen.chat_id(), reason)
        .reply_markup(teacher_keyboard())
        .await?;
    show_item(screen, item, origin).await
}

async fn ask_review_score(
    chat: Chat<'_>,
    submission_id: SubmissionId,
    scale: MaxScore,
    late: Option<LatePenalty>,
    origin: ReviewOrigin,
) -> HandlerResult {
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score: scale,
            late,
            origin,
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

struct CommentStep {
    submission_id: SubmissionId,
    decision: ReviewDecision,
    kind: SubmissionKind,
    grade: Option<Grade>,
    return_attempt: bool,
    origin: ReviewOrigin,
}

async fn ask_review_comment(chat: Chat<'_>, step: CommentStep) -> HandlerResult {
    let score = match step.kind {
        SubmissionKind::Regular => step.grade.map(|grade| ReviewScore {
            value: grade.raw_score(),
            max_score: grade.max_score(),
        }),
        SubmissionKind::Makeup => None,
    };
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id: step.submission_id,
            decision: step.decision,
            score,
            return_attempt: step.return_attempt,
            origin: step.origin,
        }))
        .await?;
    let decision = if step.return_attempt {
        texts::REWORK_DECISION.to_owned()
    } else {
        texts::review_decision(step.decision, step.kind, step.grade)
    };
    chat.bot()
        .send_message(chat.chat_id(), decision)
        .reply_markup(review_decision_keyboard(step.submission_id))
        .await?;
    chat.ask(texts::REVIEW_COMMENT_PROMPT).await
}

pub(super) async fn change_decision(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
    submission_id: SubmissionId,
    state: &BotState,
) -> HandlerResult {
    let Some(origin) = state.awaiting_review_comment(submission_id) else {
        return screen.stale().await;
    };
    let Some(item) = reload_item(screen, telegram_user_id, submission_id).await? else {
        return Ok(());
    };
    let target = review_target(&item);
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id,
            target,
            origin,
        }))
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::REVIEW_DECISION_RESET)
        .reply_markup(teacher_keyboard())
        .await?;
    screen
        .bot()
        .send_message(
            screen.chat_id(),
            review_card_summary(&item, target, screen.timezone()),
        )
        .reply_markup(item_keyboard(&item, screen.now()))
        .await?;
    Ok(())
}

pub(super) async fn handle_review_score(
    screen: Screen<'_>,
    submission_id: SubmissionId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((max_score, late, origin)) = state.awaiting_review_score(submission_id) else {
        return screen.stale().await;
    };
    let max_score = max_score.get();
    let score = match choice {
        AmountChoice::Preset(score) if score <= max_score => score,
        AmountChoice::Preset(_) | AmountChoice::Manual => {
            return screen.ask(&texts::score_prompt(max_score)).await;
        }
    };
    score_chosen(
        screen.chat(),
        submission_id,
        Grade::new(score, max_score, late),
        origin,
    )
    .await
}

async fn score_chosen(
    chat: Chat<'_>,
    submission_id: SubmissionId,
    grade: Grade,
    origin: ReviewOrigin,
) -> HandlerResult {
    ask_review_comment(
        chat,
        CommentStep {
            submission_id,
            decision: ReviewDecision::Accept,
            kind: SubmissionKind::Regular,
            grade: Some(grade),
            return_attempt: false,
            origin,
        },
    )
    .await
}

fn parse_score(text: &str, max_score: u16) -> Option<u16> {
    let score = parse_number(text)?;
    (score <= max_score).then_some(score)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::{
        Assignment, CourseMakeup, DeductedFrom, MakeupRequest, MakeupTerms, Submission,
        TeachingCourse, User,
    };
    use crate::test_support::{
        GROUP_CODE, SUBJECT_NAME, assignment, course_for, history, student, student_group, subject,
        submission, t0, teacher, vladivostok,
    };

    fn people() -> (TeachingCourse, User, User) {
        let teacher = teacher(1);
        let group = student_group(GROUP_CODE);
        (course_for(&group, &teacher), teacher, student(2, &group))
    }

    fn item(assignment: Assignment, submission: Submission, student: User) -> ReviewQueueItem {
        ReviewQueueItem {
            submission,
            assignment,
            subject: subject(SUBJECT_NAME),
            student,
            group_code: GROUP_CODE.to_owned(),
            previous_attempts: Vec::new(),
            earlier_attempts_hidden: false,
            makeup: None,
            history: Default::default(),
        }
    }

    fn makeup_of(assignment: &Assignment, student: &User) -> Submission {
        let window = CourseMakeup::open(t0() + Duration::days(60), 3, t0()).unwrap();
        submission(assignment, student)
            .submitted_at(assignment.closes_at() + Duration::days(1))
            .makeup(MakeupRequest {
                course: Some(window),
                consent: true,
                balance: window.balance(0, 0),
            })
            .build()
    }

    #[test]
    fn a_late_submission_is_reviewed_with_the_penalty_of_its_sending_moment() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher)
            .scored(5)
            .late_policy(7, 1, 4, DeductedFrom::WorkScore)
            .published()
            .build();
        let sent = submission(&work, &student)
            .submitted_at(work.deadline_at() + Duration::days(8))
            .build();

        assert_eq!(
            review_target(&item(work, sent, student)),
            ReviewTarget::Regular {
                grading: Grading::scored(5).unwrap(),
                late: LatePenalty::restore(2, 2, DeductedFrom::WorkScore),
            }
        );
    }

    #[test]
    fn a_makeup_submission_is_reviewed_as_a_makeup() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher)
            .scored(10)
            .makeup_credit(6)
            .published()
            .build();
        let makeup = makeup_of(&work, &student);

        assert_eq!(
            review_target(&item(work, makeup, student)),
            ReviewTarget::Makeup
        );
    }

    #[test]
    fn the_review_card_names_the_local_moment_the_work_was_sent() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher).published().build();
        let sent = submission(&work, &student)
            .submitted_at(t0() + Duration::hours(15) + Duration::minutes(7))
            .build();
        let item = item(work, sent, student);

        let summary = review_card_summary(&item, review_target(&item), &vladivostok());

        assert!(summary.contains("02.01.2026 01:07"), "{summary}");
    }

    #[test]
    fn the_card_marks_the_last_allowed_regular_attempt() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher)
            .max_attempts(2)
            .published()
            .build();
        let sent = submission(&work, &student)
            .history(history().used(1).build())
            .build();
        let mut item = item(work, sent, student);
        item.history = history().used(1).build();

        assert_eq!(attempt_line(&item), texts::regular_attempt(2, 2));
        assert!(texts::regular_attempt(2, 2).contains("последняя"));
    }

    #[test]
    fn an_extra_attempt_moves_the_last_one_further() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher)
            .max_attempts(2)
            .published()
            .build();
        let sent = submission(&work, &student)
            .history(history().used(1).extra(1).build())
            .build();
        let mut item = item(work, sent, student);
        item.history = history().used(1).extra(1).build();

        let line = attempt_line(&item);
        assert_eq!(line, texts::regular_attempt(2, 3));
        assert!(!line.contains("последняя"), "{line}");
    }

    #[test]
    fn rework_is_offered_only_for_regular_work_while_acceptance_is_open() {
        let (course, teacher, student) = people();
        let work = assignment(&course, &teacher)
            .scored(10)
            .makeup_credit(6)
            .published()
            .build();
        let regular = item(
            work.clone(),
            submission(&work, &student).build(),
            student.clone(),
        );
        let makeup = item(work.clone(), makeup_of(&work, &student), student);
        let open = work.closes_at() - Duration::seconds(1);

        assert!(offers_rework(&regular, open));
        assert!(!offers_rework(&regular, work.closes_at()));
        assert!(!offers_rework(&makeup, open));
    }

    #[test]
    fn the_accept_button_names_what_the_acceptance_grants() {
        let (course, teacher, student) = people();
        let regular = assignment(&course, &teacher).published().build();
        let credited = assignment(&course, &teacher)
            .scored(10)
            .makeup_credit(6)
            .published()
            .build();
        let pass_fail = assignment(&course, &teacher)
            .makeup(MakeupTerms::PassFail)
            .published()
            .build();

        for (work, sent, label) in [
            (
                regular.clone(),
                submission(&regular, &student).build(),
                texts::BUTTON_ACCEPT.to_owned(),
            ),
            (
                credited.clone(),
                makeup_of(&credited, &student),
                texts::accept_makeup_button(6),
            ),
            (
                pass_fail.clone(),
                makeup_of(&pass_fail, &student),
                texts::BUTTON_ACCEPT_MAKEUP.to_owned(),
            ),
        ] {
            assert_eq!(
                accept_label(&item(work, sent, student.clone())),
                Some(label)
            );
        }
    }

    #[test]
    fn a_makeup_whose_terms_were_withdrawn_has_no_accept_button() {
        let (course, teacher, student) = people();
        let mut work = assignment(&course, &teacher)
            .scored(10)
            .makeup_credit(6)
            .published()
            .build();
        let makeup = makeup_of(&work, &student);
        work.decide_makeup(&course, &teacher, MakeupTerms::NotOffered)
            .unwrap();

        assert_eq!(accept_label(&item(work, makeup, student)), None);
    }

    #[test]
    fn a_decided_missing_or_forbidden_submission_closes_the_review() {
        for error in [
            AppError::Domain(DomainError::SubmissionIsNotPendingReview),
            AppError::NotFound {
                resource: "submission",
            },
            AppError::Domain(DomainError::PermissionDenied {
                action: "review submission",
            }),
        ] {
            assert!(review_closed(&error), "{error:?}");
        }
    }

    #[test]
    fn a_changed_grading_or_a_transient_failure_keeps_the_review_open() {
        for error in [
            AppError::Domain(DomainError::ScoreRequired),
            AppError::Domain(DomainError::MakeupNotOffered),
            AppError::AssignmentChangedConcurrently,
            AppError::Overloaded,
        ] {
            assert!(!review_closed(&error), "{error:?}");
        }
    }

    #[test]
    fn a_typed_score_is_accepted_from_zero_up_to_the_scale_maximum() {
        assert_eq!(parse_score("0", 10), Some(0));
        assert_eq!(parse_score("10", 10), Some(10));
        assert_eq!(parse_score("11", 10), None);
    }

    #[test]
    fn a_typed_score_is_a_plain_integer_with_optional_surrounding_spaces() {
        assert_eq!(parse_score(" 7 ", 10), Some(7));
        for text in ["", "семь", "7.5", "+7", "-1", "7 из 10"] {
            assert_eq!(parse_score(text, 10), None, "{text}");
        }
    }
}
