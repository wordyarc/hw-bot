use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;

use super::input::require_text_within;
use super::reset_dialogue;
use crate::app::{AppError, ErrorCode, StudentAssignment, SubmitHomework};
use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::{app_error_message, log_handler_error, optional_stat};
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{
    list_text, makeup_credit_terms, paginated_keyboard, reopen_assignment_keyboard,
    send_message_chunks, student_assignment_keyboard, student_assignments_keyboard,
    student_grading_lines, student_keyboard, student_menu, submission_history_keyboard,
};
use crate::domain::{AssignmentId, MakeupOffer, StoredFileMetadata, Submission, USER_TEXT_MAX_LEN};
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
        text,
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
    let assignments = services.assignments.list_for_student(actor, page).await?;

    let now = Utc::now();
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
            let makeup = available(item.makeup)
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
        lines.join("\n"),
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
    let details = services
        .assignments
        .find_for_student(actor, assignment_id)
        .await?;
    let assignment = &details.assignment.assignment;
    let history = details.assignment.history;
    let makeup = details.assignment.makeup;
    let now = Utc::now();
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
        Some(texts::BUTTON_SUBMIT)
    } else {
        available(makeup).map(|_| texts::BUTTON_SUBMIT_MAKEUP)
    };
    let keyboard =
        student_assignment_keyboard(assignment, details.latest_submission.is_some(), submit);

    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, texts::ASSIGNMENT_BODY_HEADER)
        .await?;
    send_message_chunks(bot, chat_id, assignment.body().as_str().to_owned(), None).await?;
    if let Some(submission) = &details.latest_submission {
        send_submission(screen, submission).await?;
    }
    bot.send_message(chat_id, texts::CHOOSE_ACTION)
        .reply_markup(keyboard)
        .await?;
    if let Err(error) = send_assignment_files(screen, assignment_id, &details.files).await {
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

async fn send_assignment_files(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    files: &[StoredFileMetadata],
) -> HandlerResult {
    if files.is_empty() {
        return Ok(());
    }
    screen
        .bot()
        .send_message(screen.chat_id(), texts::FILES_ATTACHED_HEADER)
        .await?;
    for metadata in files {
        let file = screen
            .services()
            .files
            .open_assignment_file_for_student(screen.actor(), assignment_id, metadata.id())
            .await?;
        screen
            .transfers()
            .send(screen.chat_id(), &screen.services().files, &file)
            .await?;
    }
    Ok(())
}

async fn send_submission(screen: Screen<'_>, submission: &Submission) -> HandlerResult {
    let grade = texts::grade_line(submission.kind(), submission.grade());
    let text = texts::attempt(
        submission.attempt_number(),
        texts::submission_status(submission.status()),
        grade.as_deref(),
        submission.content().as_str(),
        submission.review_comment().map(|comment| comment.as_str()),
    );
    send_message_chunks(screen.bot(), screen.chat_id(), text, None).await
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

fn available(makeup: Option<MakeupOffer>) -> Option<MakeupOffer> {
    makeup.filter(|makeup| makeup.balance.remaining() > 0)
}

fn submission_offer(
    item: &StudentAssignment,
    shown_at: DateTime<Utc>,
) -> Result<Option<MakeupOffer>, AppError> {
    let makeup = available(item.makeup);
    if makeup.is_none()
        && let Err(refused) = item
            .history
            .ensure_regular_attempt(&item.assignment, shown_at)
    {
        let reason = item
            .makeup
            .and_then(|offered| offered.balance.ensure_available().err())
            .unwrap_or(refused);
        return Err(reason.into());
    }
    Ok(makeup)
}

pub(super) async fn start_submission(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    shown_at: DateTime<Utc>,
) -> HandlerResult {
    let admitted = screen
        .services()
        .assignments
        .find_for_student(screen.actor(), assignment_id)
        .await
        .and_then(|details| {
            submission_offer(&details.assignment, shown_at).map(|makeup| (details, makeup))
        });
    let (details, makeup) = match admitted {
        Ok(admitted) => admitted,
        Err(error) if error.code().is_server_failure() => return Err(error.into()),
        Err(error) => {
            screen
                .bot()
                .send_message(screen.chat_id(), app_error_message(error.code()))
                .reply_markup(student_keyboard())
                .await?;
            return Ok(());
        }
    };
    let item = &details.assignment;
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingSubmission {
            assignment_id,
            makeup: makeup.is_some(),
        }))
        .await?;
    match makeup {
        Some(makeup) => {
            let terms = makeup_credit_terms(makeup.credit, item.assignment.grading());
            screen
                .ask(&texts::makeup_prompt(&terms, makeup.balance.remaining()))
                .await
        }
        None => screen.ask(texts::SUBMISSION_PROMPT).await,
    }
}

pub(super) async fn submit_homework_text(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
    makeup: bool,
) -> HandlerResult {
    let bot = step.bot();
    let chat_id = step.chat_id();
    let Some(text) = require_text_within(
        bot,
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
            allow_makeup: makeup,
        })
        .await;
    match submitted {
        Ok(submitted) => {
            reset_dialogue(step.dialogue()).await?;
            let confirmation = match submitted.makeup_balance {
                Some(balance) => texts::makeup_queued(balance.remaining()),
                None => texts::submission_queued(),
            };
            bot.send_message(chat_id, confirmation)
                .reply_markup(student_keyboard())
                .await?;
            Ok(())
        }
        Err(error)
            if matches!(
                error.code(),
                ErrorCode::DeadlineExceeded | ErrorCode::SubmissionClosed
            ) =>
        {
            reset_dialogue(step.dialogue()).await?;
            bot.send_message(chat_id, app_error_message(error.code()))
                .reply_markup(student_keyboard())
                .await?;
            show_assignment(screen, assignment_id).await
        }
        Err(error) if error.code().is_server_failure() => Err(error.into()),
        Err(error) => {
            reset_dialogue(step.dialogue()).await?;
            bot.send_message(chat_id, app_error_message(error.code()))
                .reply_markup(student_keyboard())
                .await?;
            Ok(())
        }
    }
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
