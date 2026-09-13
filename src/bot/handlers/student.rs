use super::input::{ensure_text_limit, require_text};
use super::{reset_dialogue, resolve_actor};
use crate::app::SubmitHomework;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::{Deps, Screen, TextStep};
use crate::bot::texts;
use crate::bot::views::paginated_keyboard;
use crate::bot::views::student_keyboard;
use crate::domain::USER_TEXT_MAX_LEN;
use crate::pagination::PageRequest;
use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::domain::{AssignmentId, AssignmentStatus};

use crate::bot::views::{
    button_label, deadline_left, grade_line, grading_lines, send_message_chunks, student_menu,
    submission_status,
};
use crate::bot::{FileTransfers, HandlerResult};

pub(super) async fn show_queue_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        ..
    } = screen;
    let services = &context.services;
    let timezone = &context.timezone;
    let places = services.submissions.pending_queue(actor, page).await?;

    if places.items.is_empty() && page.page == 0 {
        bot.send_message(chat_id, crate::bot::texts::STUDENT_QUEUE_EMPTY)
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut text = String::from("⏳ Ваши работы на проверке:\n");
    for place in &places.items {
        text.push_str(&format!(
            "\n📘 {} · {}\nотправлено {} · {}\n",
            place.subject_name,
            place.assignment_title,
            timezone.format_short(place.submitted_at),
            texts::queue_ahead(place.position),
        ));
    }

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

#[derive(Default)]
struct SubjectHeadings {
    current: Option<String>,
}

impl SubjectHeadings {
    fn push(&mut self, text: &mut String, subject: &str) {
        if self.current.as_deref() == Some(subject) {
            return;
        }
        text.push_str(&format!("\n📘 {subject}\n"));
        self.current = Some(subject.to_owned());
    }
}

pub(super) async fn show_assignments_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        ..
    } = screen;
    let services = &context.services;
    let timezone = &context.timezone;
    let assignments = services.assignments.list_for_student(actor, page).await?;

    if assignments.items.is_empty() && page.page == 0 {
        bot.send_message(chat_id, "📚 Опубликованных заданий пока нет.")
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let now = Utc::now();
    let mut text = String::from("📚 Ваши задания:\n");
    let mut rows = Vec::with_capacity(assignments.items.len());
    let mut subjects = SubjectHeadings::default();
    for item in &assignments.items {
        let assignment = &item.assignment;
        let subject = item.subject.name().as_str();
        let remaining = item.history.remaining_attempts_for(assignment);
        let status = if item.history.has_accepted_submission() {
            "✅ принято"
        } else if item.history.has_pending_submission() {
            "⏳ на проверке"
        } else {
            "✍️ не сдано"
        };
        subjects.push(&mut text, subject);
        text.push_str(&format!(
            "\n{}\n{status} · {} {} ({}){}\nосталось попыток: {}\n",
            assignment.title().as_str(),
            if assignment.late_policy().is_some() {
                texts::NO_PENALTY_UNTIL
            } else {
                texts::SUBMIT_UNTIL
            },
            timezone.format(assignment.deadline_at()),
            deadline_left(assignment.deadline_at(), now),
            assignment
                .late_policy()
                .map(|_| format!(
                    "\n{} {}",
                    texts::SUBMISSIONS_CLOSE_LABEL,
                    timezone.format(assignment.closes_at())
                ))
                .unwrap_or_default(),
            remaining,
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            button_label(&format!("{subject} · {}", assignment.title().as_str())),
            format!(
                "{}{}",
                crate::bot::callbacks::ASSIGNMENT_PREFIX,
                assignment.id().as_uuid()
            ),
        )]);
    }

    send_message_chunks(
        bot,
        chat_id,
        text,
        Some(paginated_keyboard(
            InlineKeyboardMarkup::new(rows),
            PageTarget::Assignments,
            assignments.page,
            assignments.next_page,
        )),
    )
    .await
}

pub(super) async fn show_archive_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        ..
    } = screen;
    let services = &context.services;
    let assignments = services
        .assignments
        .list_archived_for_student(actor, page)
        .await?;

    if assignments.items.is_empty() && page.page == 0 {
        bot.send_message(chat_id, "🗄 Архив заданий пока пуст.")
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut text = String::from("🗄 Архив заданий:\n");
    let mut rows = Vec::with_capacity(assignments.items.len());
    let mut subjects = SubjectHeadings::default();
    for item in &assignments.items {
        let assignment = &item.assignment;
        let subject = item.subject.name().as_str();
        subjects.push(&mut text, subject);
        text.push_str(&format!(
            "\n{}\nиспользовано попыток: {}\n",
            assignment.title().as_str(),
            item.history.used_attempts(),
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            button_label(&format!("{subject} · {}", assignment.title().as_str())),
            format!(
                "{}{}",
                crate::bot::callbacks::ASSIGNMENT_PREFIX,
                assignment.id().as_uuid()
            ),
        )]);
    }

    send_message_chunks(
        bot,
        chat_id,
        text,
        Some(paginated_keyboard(
            InlineKeyboardMarkup::new(rows),
            PageTarget::Archive,
            assignments.page,
            assignments.next_page,
        )),
    )
    .await
}

pub(super) async fn show_assignment(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    transfers: &FileTransfers,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        ..
    } = screen;
    let services = &context.services;
    let timezone = &context.timezone;
    let details = services
        .assignments
        .find_for_student(actor, assignment_id)
        .await?;
    let assignment = &details.assignment.assignment;
    let history = details.assignment.history;
    let mut summary = format!(
        "📘 {}\n{}\n{}\n🔁 Попытки: использовано {}, доступно {}, осталось {}",
        details.assignment.subject.name().as_str(),
        assignment.title().as_str(),
        grading_lines(assignment, Utc::now(), timezone),
        history.used_attempts(),
        history.allowed_attempts_for(assignment),
        history.remaining_attempts_for(assignment),
    );
    if history.has_pending_submission()
        && let Ok(Some(place)) = services
            .submissions
            .queue_place(actor, assignment_id)
            .await
            .inspect_err(|error| {
                tracing::warn!(
                    code = error.code().as_str(),
                    "student queue position is unavailable"
                );
            })
    {
        summary.push_str(&format!(
            "\n⏳ Работа на проверке.\n{}",
            texts::queue_ahead(place.position),
        ));
    }

    let mut rows = Vec::new();
    if !details.submissions.is_empty() {
        rows.push(vec![InlineKeyboardButton::callback(
            "🕘 История сдач",
            PageTarget::History(assignment_id).callback(0),
        )]);
    }
    if assignment.status() == AssignmentStatus::Published {
        rows.push(vec![InlineKeyboardButton::callback(
            "📤 Сдать работу",
            format!(
                "{}{}",
                crate::bot::callbacks::SUBMIT_PREFIX,
                assignment.id().as_uuid()
            ),
        )]);
    }
    let (back_label, back_callback) = if assignment.status() == AssignmentStatus::Archived {
        ("⬅️ К архиву", "menu:archive")
    } else {
        ("⬅️ К списку заданий", "menu:assignments")
    };
    rows.push(vec![InlineKeyboardButton::callback(
        back_label,
        back_callback,
    )]);

    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, "📄 Текст задания:").await?;
    bot.send_message(chat_id, assignment.body().as_str())
        .await?;
    if !details.files.is_empty() {
        bot.send_message(chat_id, "📎 Вложения задания:").await?;
        for metadata in &details.files {
            let transfer = transfers.acquire().await?;
            transfer
                .run(async {
                    let file = services
                        .files
                        .open_assignment_file_for_student(actor, assignment.id(), metadata.id())
                        .await?;
                    transfers.send(chat_id, file).await
                })
                .await?;
        }
    }
    for submission in &details.submissions {
        send_submission(screen, submission).await?;
    }
    bot.send_message(chat_id, "Выберите действие:")
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

async fn send_submission(
    screen: Screen<'_>,
    submission: &crate::domain::Submission,
) -> HandlerResult {
    let Screen { bot, chat_id, .. } = screen;
    bot.send_message(
        chat_id,
        format!(
            "Попытка {}: {}{}",
            submission.attempt_number(),
            submission_status(submission.status()),
            submission
                .grade()
                .map(|grade| format!("\n{}", grade_line(grade)))
                .unwrap_or_default(),
        ),
    )
    .await?;
    bot.send_message(chat_id, submission.content().as_str())
        .await?;
    if let Some(comment) = submission.review_comment() {
        bot.send_message(chat_id, "💬 Комментарий преподавателя:")
            .await?;
        bot.send_message(chat_id, comment.as_str()).await?;
    }
    Ok(())
}

pub(super) async fn show_history(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    page: PageRequest,
) -> HandlerResult {
    let history = screen
        .context
        .services
        .assignments
        .submission_history(screen.actor, assignment_id, page)
        .await?;
    screen
        .bot
        .send_message(
            screen.chat_id,
            format!(
                "📘 {} · {}\n🕘 История сдач: от новых к старым",
                history.subject_name, history.assignment_title
            ),
        )
        .await?;
    let submissions = history.submissions;
    for submission in &submissions.items {
        send_submission(screen, submission).await?;
    }
    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "⬅️ К заданию",
        format!(
            "{}{}",
            crate::bot::callbacks::ASSIGNMENT_PREFIX,
            assignment_id.as_uuid()
        ),
    )]]);
    screen
        .bot
        .send_message(screen.chat_id, "Выберите действие:")
        .reply_markup(paginated_keyboard(
            keyboard,
            PageTarget::History(assignment_id),
            submissions.page,
            submissions.next_page,
        ))
        .await?;
    Ok(())
}

pub(super) async fn submit_homework_text(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
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
    if !ensure_text_limit(
        bot,
        message.chat.id,
        text,
        USER_TEXT_MAX_LEN,
        "Текст работы",
    )
    .await?
    {
        return Ok(());
    }
    let Some(actor) =
        resolve_actor(bot, message.chat.id, telegram_user_id, services, dialogue).await?
    else {
        return Ok(());
    };
    services
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id,
            actor,
            content: text.to_owned(),
            submitted_at: Utc::now(),
        })
        .await?;

    reset_dialogue(dialogue).await?;
    bot.send_message(message.chat.id, texts::submission_queued())
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
