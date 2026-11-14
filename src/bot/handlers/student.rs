use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;

use super::input::require_text_within;
use super::reset_dialogue;
use crate::app::{
    AppError, ErrorCode, OpenedFile, PendingWithdrawal, StudentAssignment, SubmitHomework,
    WithdrawSubmission,
};
use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::{BotError, app_error_message, log_handler_error, optional_stat};
use crate::bot::state::{BotState, FlowState, MakeupConsent};
use crate::bot::texts;
use crate::bot::views::{
    AssignmentAction, list_text, makeup_credit_terms, paginated_keyboard,
    reopen_assignment_keyboard, resubmission_line, send_message_chunks,
    student_assignment_keyboard, student_assignments_keyboard, student_grading_lines,
    student_keyboard, student_menu, submission_history_keyboard, withdrawal_confirmation_keyboard,
    withdrawal_hint_keyboard,
};
use crate::domain::{
    AssignmentId, MakeupOffer, Submission, SubmissionId, SubmissionKind, SubmissionStart,
    USER_TEXT_MAX_LEN,
};
use crate::pagination::{Page, PageRequest};

pub(super) async fn show_queue_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let services = screen.services();
    let timezone = screen.timezone();
    let places = services.submissions.pending_queue(actor, page).await?;

    if places.items.is_empty() && page.page == 0 {
        bot.send_message(chat_id, texts::STUDENT_QUEUE_EMPTY)
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let entries = places.items.iter().map(|place| {
        texts::queue_entry(
            &place.subject_name,
            &place.assignment_title,
            &timezone.format_short(place.submitted_at),
            &texts::queue_ahead(place.position),
        )
    });
    let text = list_text(texts::STUDENT_QUEUE_HEADER, entries);

    send_message_chunks(
        bot,
        chat_id,
        &text,
        Some(paginated_keyboard(
            student_menu(),
            PageTarget::Queue,
            places.page,
            places.next_page,
        )),
    )
    .await
}

pub(super) async fn show_assignments_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let actor = screen.actor();
    let services = screen.services();
    let timezone = screen.timezone();
    let now = screen.now();
    let assignments = services.assignments.list_for_student(actor, page).await?;

    show_assignment_list(
        screen.chat(),
        assignments,
        texts::ASSIGNMENTS_HEADER,
        texts::NO_ASSIGNMENTS,
        PageTarget::Assignments,
        |item| {
            let assignment = &item.assignment;
            let status = if item.history.has_accepted_submission() {
                texts::PROGRESS_ACCEPTED
            } else if item.history.has_pending_submission() {
                texts::PROGRESS_PENDING
            } else {
                texts::PROGRESS_NOT_SUBMITTED
            };
            let until = format!(
                "{} {}",
                if assignment.late_policy().is_some() {
                    texts::NO_PENALTY_UNTIL
                } else {
                    texts::SUBMIT_UNTIL
                },
                timezone.format(assignment.deadline_at())
            );
            let closing = assignment
                .late_policy()
                .map(|_| texts::closing_line(&timezone.format(assignment.closes_at())));
            let makeup = item
                .makeup_offer_at(now)
                .filter(MakeupOffer::has_attempts)
                .map(|makeup| texts::makeup_until_line(&timezone.format(makeup.until)));
            texts::assignment_entry(
                assignment.title().as_str(),
                status,
                &until,
                &texts::deadline_left(assignment.deadline_at(), now),
                closing.as_deref(),
                makeup.as_deref(),
                item.history.remaining_attempts_for(assignment),
            )
        },
    )
    .await
}

pub(super) async fn show_archive_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let actor = screen.actor();
    let services = screen.services();
    let assignments = services
        .assignments
        .list_archived_for_student(actor, page)
        .await?;

    show_assignment_list(
        screen.chat(),
        assignments,
        texts::ARCHIVE_HEADER,
        texts::ARCHIVE_EMPTY,
        PageTarget::Archive,
        |item| {
            texts::archive_entry(
                item.assignment.title().as_str(),
                item.history.used_attempts(),
            )
        },
    )
    .await
}

async fn show_assignment_list(
    chat: Chat<'_>,
    assignments: Page<StudentAssignment>,
    title: &str,
    empty: &str,
    target: PageTarget,
    describe: impl Fn(&StudentAssignment) -> String,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    if assignments.items.is_empty() && assignments.page == 0 {
        bot.send_message(chat_id, empty)
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut lines = vec![title.to_owned()];
    let mut current: Option<&str> = None;
    for item in &assignments.items {
        let subject = item.subject.name().as_str();
        if current != Some(subject) {
            lines.push(String::new());
            lines.push(texts::subject_header(subject));
            current = Some(subject);
        }
        lines.push(String::new());
        lines.push(describe(item));
    }

    send_message_chunks(
        bot,
        chat_id,
        &lines.join("\n"),
        Some(student_assignments_keyboard(
            &assignments.items,
            target,
            assignments.page,
            assignments.next_page,
        )),
    )
    .await
}

pub(super) async fn show_assignment(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let services = screen.services();
    let timezone = screen.timezone();
    let now = screen.now();
    let details = services
        .assignments
        .find_for_student(actor, assignment_id)
        .await?;
    let assignment = &details.assignment.assignment;
    let history = details.assignment.history;
    let makeup = details.assignment.makeup_offer_at(now);
    let mut summary = texts::assignment_card(
        details.assignment.subject.name().as_str(),
        assignment.title().as_str(),
        &student_grading_lines(assignment, makeup, now, timezone),
        history.used_attempts(),
        history.allowed_attempts_for(assignment),
        history.remaining_attempts_for(assignment),
    );
    if history.has_pending_submission()
        && let Some(place) = optional_stat(
            services.submissions.queue_place(actor, assignment_id).await,
            "student_queue_place",
        )
        .flatten()
    {
        summary.push('\n');
        summary.push_str(&texts::pending_place(&texts::queue_ahead(place.position)));
    }

    let submit = if history.accepts_new_attempt_for(assignment, now) {
        Some(SubmissionKind::Regular)
    } else {
        makeup
            .filter(MakeupOffer::has_attempts)
            .map(|_| SubmissionKind::Makeup)
    };
    let action = details
        .latest_submission
        .as_ref()
        .filter(|latest| latest.is_pending_review())
        .map(|pending| AssignmentAction::Withdraw(pending.id()))
        .or(submit.map(AssignmentAction::Submit));
    let keyboard =
        student_assignment_keyboard(assignment, details.latest_submission.is_some(), action);

    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, texts::ASSIGNMENT_BODY_HEADER)
        .await?;
    send_message_chunks(bot, chat_id, assignment.body().as_str(), None).await?;
    if let Some(submission) = &details.latest_submission {
        send_submission(screen, submission).await?;
    }
    bot.send_message(chat_id, texts::CHOOSE_ACTION)
        .reply_markup(keyboard)
        .await?;
    if let Err(error) = send_assignment_files(screen, &details.files).await {
        if error.is_invalid_token() {
            return Err(error);
        }
        log_handler_error(&error);
        bot.send_message(chat_id, texts::ASSIGNMENT_FILES_SEND_FAILED)
            .reply_markup(reopen_assignment_keyboard(assignment_id))
            .await?;
    }
    Ok(())
}

async fn send_assignment_files(screen: Screen<'_>, files: &[OpenedFile]) -> HandlerResult {
    if files.is_empty() {
        return Ok(());
    }
    screen
        .bot()
        .send_message(screen.chat_id(), texts::FILES_ATTACHED_HEADER)
        .await?;
    for file in files {
        screen
            .transfers()
            .send(screen.chat_id(), &screen.services().files, file)
            .await?;
    }
    Ok(())
}

async fn send_submission(screen: Screen<'_>, submission: &Submission) -> HandlerResult {
    let grade = texts::grade_line(submission.kind(), submission.grade());
    let status = match submission.withdrawn_at() {
        Some(at) => texts::withdrawn_status(&screen.timezone().format_moment(at)),
        None => texts::submission_status(submission.status()).to_owned(),
    };
    let text = texts::attempt(
        submission.attempt_number(),
        &status,
        grade.as_deref(),
        submission.content().as_str(),
        submission.review_comment().map(|comment| comment.as_str()),
    );
    send_message_chunks(screen.bot(), screen.chat_id(), &text, None).await
}

pub(super) async fn show_history(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    page: PageRequest,
) -> HandlerResult {
    let history = screen
        .services()
        .assignments
        .submission_history(screen.actor(), assignment_id, page)
        .await?;
    screen
        .bot()
        .send_message(
            screen.chat_id(),
            texts::history_header(&history.subject_name, &history.assignment_title),
        )
        .await?;
    let submissions = history.submissions;
    for submission in &submissions.items {
        send_submission(screen, submission).await?;
    }
    screen
        .bot()
        .send_message(screen.chat_id(), texts::CHOOSE_ACTION)
        .reply_markup(submission_history_keyboard(
            assignment_id,
            submissions.page,
            submissions.next_page,
        ))
        .await?;
    Ok(())
}

pub(super) async fn start_submission(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    button: SubmissionKind,
    shown_at: DateTime<Utc>,
) -> HandlerResult {
    let now = screen.now();
    let started = screen
        .services()
        .assignments
        .find_for_student(screen.actor(), assignment_id)
        .await
        .and_then(|details| {
            let item = details.assignment;
            item.submission_start(button, shown_at, now)
                .map(|start| (item, start))
        });
    let (item, start) = match started {
        Ok(started) => started,
        Err(error) if error.code().is_server_failure() => return Err(error.into()),
        Err(error) => {
            return send_with_menu(screen.chat(), app_error_message(error.code())).await;
        }
    };
    let timezone = screen.timezone();
    let makeup = match start {
        SubmissionStart::Regular => {
            let prompt = if item.history.accepts_new_attempt_for(&item.assignment, now) {
                texts::SUBMISSION_PROMPT.to_owned()
            } else {
                texts::closed_submission_prompt(
                    &timezone.format_closed_since(item.assignment.closes_at()),
                )
            };
            screen.ask(&prompt).await?;
            MakeupConsent::Withheld
        }
        SubmissionStart::Makeup { offer, consented } => {
            let terms = makeup_credit_terms(offer.credit, item.assignment.grading());
            let remaining = offer.balance.remaining();
            let prompt = if item.makeup_offer_at(now).is_some() {
                texts::makeup_prompt(&terms, remaining)
            } else {
                texts::closed_makeup_prompt(
                    &terms,
                    remaining,
                    &timezone.format_closed_since(offer.until),
                )
            };
            let prompted = screen.chat().ask_message(&prompt).await?;
            if consented {
                MakeupConsent::Given
            } else {
                MakeupConsent::Prompted(prompted)
            }
        }
    };
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingSubmission {
            assignment_id,
            makeup,
        }))
        .await?;
    Ok(())
}

pub(super) async fn submit_homework_text(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
    makeup: MakeupConsent,
) -> HandlerResult {
    let Some(text) = require_text_within(
        step.bot(),
        step.message(),
        USER_TEXT_MAX_LEN,
        texts::FIELD_SUBMISSION,
    )
    .await?
    else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let submitted = step
        .services()
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id,
            actor: screen.actor(),
            content: text.into_owned(),
            submitted_at: step.message().date,
            allow_makeup: makeup.covers(step.message().id),
        })
        .await;
    let submitted = match submitted {
        Err(error) if error.code().is_server_failure() => return Err(error.into()),
        submitted => submitted,
    };
    reset_dialogue(step.dialogue()).await?;
    match submitted {
        Ok(submitted) => {
            let confirmation = match submitted.makeup_balance {
                Some(balance) => texts::makeup_queued(balance.remaining()),
                None => texts::submission_queued(),
            };
            send_with_menu(screen.chat(), confirmation).await?;
            let hint = screen
                .bot()
                .send_message(screen.chat_id(), texts::WITHDRAWAL_HINT)
                .reply_markup(withdrawal_hint_keyboard(submitted.submission.id()))
                .await;
            if let Err(error) = hint {
                let error = BotError::from(error);
                if error.is_invalid_token() {
                    return Err(error);
                }
                log_handler_error(&error);
            }
            Ok(())
        }
        Err(error) => {
            send_with_menu(screen.chat(), app_error_message(error.code())).await?;
            if matches!(
                error.code(),
                ErrorCode::DeadlineExceeded | ErrorCode::SubmissionClosed
            ) {
                return show_assignment(screen, assignment_id).await;
            }
            Ok(())
        }
    }
}

pub(super) async fn withdrawal_step(
    screen: Screen<'_>,
    submission_id: SubmissionId,
    shown_confirmation: Option<&str>,
) -> HandlerResult {
    let pending = screen
        .services()
        .submissions
        .find_for_withdrawal(screen.actor(), submission_id)
        .await?;
    if let Err(refused) = pending.submission.ensure_pending_review() {
        return refuse_withdrawal(screen, &pending, refused.into()).await;
    }
    let confirmation = withdrawal_confirmation(screen, &pending);
    if shown_confirmation != Some(confirmation.as_str()) {
        return send_withdrawal_confirmation(screen, &pending, confirmation).await;
    }

    let withdrawn = screen
        .services()
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id,
            actor: screen.actor(),
            withdrawn_at: screen.now(),
        })
        .await;
    match withdrawn {
        Ok(_) => {
            screen
                .bot()
                .send_message(screen.chat_id(), texts::WITHDRAWN)
                .await?;
        }
        Err(error) if error.code().is_server_failure() => return Err(error.into()),
        Err(error) => return refuse_withdrawal(screen, &pending, error).await,
    }
    show_assignment(screen, pending.submission.assignment_id()).await
}

async fn refuse_withdrawal(
    screen: Screen<'_>,
    pending: &PendingWithdrawal,
    refusal: AppError,
) -> HandlerResult {
    screen.fail(refusal).await?;
    show_assignment(screen, pending.submission.assignment_id()).await
}

fn withdrawal_confirmation(screen: Screen<'_>, pending: &PendingWithdrawal) -> String {
    let assignment = &pending.assignment.assignment;
    texts::withdrawal_confirmation(
        pending.assignment.subject.name().as_str(),
        assignment.title().as_str(),
        &resubmission_line(
            &pending.resubmission_at(screen.now()),
            assignment,
            screen.timezone(),
        ),
    )
}

async fn send_withdrawal_confirmation(
    screen: Screen<'_>,
    pending: &PendingWithdrawal,
    confirmation: String,
) -> HandlerResult {
    screen
        .bot()
        .send_message(screen.chat_id(), confirmation)
        .reply_markup(withdrawal_confirmation_keyboard(
            pending.submission.id(),
            pending.submission.assignment_id(),
        ))
        .await?;
    Ok(())
}

async fn send_with_menu(chat: Chat<'_>, text: impl Into<String>) -> HandlerResult {
    chat.bot()
        .send_message(chat.chat_id(), text)
        .reply_markup(student_keyboard())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queue_without_reviewers_does_not_show_a_made_up_position() {
        assert!(texts::queue_ahead(6).contains('5'));
        assert_eq!(texts::queue_ahead(1), texts::QUEUE_FIRST);
        assert_eq!(texts::queue_ahead(0), texts::QUEUE_NO_TEACHER);
    }
}
