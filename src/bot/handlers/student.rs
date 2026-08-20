use super::{Deps, TextStep, ensure_text_limit, require_text, reset_dialogue, resolve_actor};
use crate::app::SubmitHomework;
use crate::bot::texts;
use crate::bot::views::student_keyboard;
use crate::domain::USER_TEXT_MAX_LEN;
use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::domain::{AssignmentId, AssignmentStatus};

use super::Screen;
use crate::bot::errors::send_app_error;
use crate::bot::views::{
    button_label, deadline_left, grade_line, grading_lines, send_message_chunks, student_menu,
    submission_status,
};
use crate::bot::{FileTransfers, HandlerResult};

pub(super) async fn show_queue(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        timezone,
        ..
    } = screen;
    let places = match services.submissions.pending_queue(actor).await {
        Ok(places) => places,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if places.is_empty() {
        bot.send_message(chat_id, crate::bot::texts::STUDENT_QUEUE_EMPTY)
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut text = String::from("⏳ Ваши работы на проверке:\n");
    for place in &places {
        text.push_str(&format!(
            "\n📘 {} · {}\nотправлено {} · {}\n",
            place.subject_name,
            place.assignment_title,
            timezone.format_short(place.submitted_at),
            queue_place(place.position, place.total),
        ));
    }

    send_message_chunks(bot, chat_id, text, Some(student_menu())).await
}

pub(super) fn queue_place(position: u32, total: u32) -> String {
    if position == 0 {
        "преподаватель ещё не назначен на курс".to_owned()
    } else {
        format!("ваш номер в очереди: {position} из {total}")
    }
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

pub(super) async fn show_assignments(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        timezone,
        ..
    } = screen;
    let assignments = match services.assignments.list_for_student(actor).await {
        Ok(assignments) => assignments,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if assignments.is_empty() {
        bot.send_message(chat_id, "📚 Опубликованных заданий пока нет.")
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let now = Utc::now();
    let mut text = String::from("📚 Ваши задания:\n");
    let mut rows = Vec::with_capacity(assignments.len());
    let mut subjects = SubjectHeadings::default();
    for item in assignments {
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
            "\n{}\n{status} · до {} ({})\nосталось попыток: {}\n",
            assignment.title().as_str(),
            timezone.format(assignment.deadline_at()),
            deadline_left(assignment.deadline_at(), now),
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

    send_message_chunks(bot, chat_id, text, Some(InlineKeyboardMarkup::new(rows))).await
}

pub(super) async fn show_archive(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        ..
    } = screen;
    let assignments = match services.assignments.list_archived_for_student(actor).await {
        Ok(assignments) => assignments,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if assignments.is_empty() {
        bot.send_message(chat_id, "🗄 Архив заданий пока пуст.")
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut text = String::from("🗄 Архив заданий:\n");
    let mut rows = Vec::with_capacity(assignments.len());
    let mut subjects = SubjectHeadings::default();
    for item in assignments {
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

    send_message_chunks(bot, chat_id, text, Some(InlineKeyboardMarkup::new(rows))).await
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
        services,
        timezone,
        ..
    } = screen;
    let details = match services
        .assignments
        .find_for_student(actor, assignment_id)
        .await
    {
        Ok(details) => details,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let assignment = &details.assignment.assignment;
    let history = details.assignment.history;
    let mut summary = format!(
        "📘 {}\n{}\n\n⏰ Дедлайн: {} ({}){}\n🔁 Попытки: использовано {}, доступно {}, осталось {}",
        details.assignment.subject.name().as_str(),
        assignment.title().as_str(),
        timezone.format(assignment.deadline_at()),
        deadline_left(assignment.deadline_at(), Utc::now()),
        grading_lines(assignment, Utc::now(), timezone),
        history.used_attempts(),
        history.allowed_attempts_for(assignment),
        history.remaining_attempts_for(assignment),
    );
    if history.has_pending_submission()
        && let Ok(places) = services.submissions.pending_queue(actor).await
        && let Some(place) = places
            .iter()
            .find(|place| place.assignment_id == assignment_id)
    {
        summary.push_str(&format!(
            "\n⏳ Работа на проверке, {}",
            queue_place(place.position, place.total),
        ));
    }

    let mut rows = Vec::new();
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
        let _permit = transfers.acquire().await?;
        for metadata in &details.files {
            match services
                .files
                .open_assignment_file_for_student(actor, assignment.id(), metadata.id())
                .await
            {
                Ok(file) => transfers.send(chat_id, file).await?,
                Err(error) => {
                    send_app_error(bot, chat_id, error).await?;
                    return Ok(());
                }
            }
        }
    }
    for submission in &details.submissions {
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
    }
    bot.send_message(chat_id, "Выберите действие:")
        .reply_markup(InlineKeyboardMarkup::new(rows))
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
    let Deps {
        services,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

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
    let Some(actor) = resolve_actor(bot, message.chat.id, telegram_user_id, services).await? else {
        return Ok(());
    };
    let result = services
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id,
            actor,
            content: text.to_owned(),
            submitted_at: Utc::now(),
        })
        .await;

    match result {
        Ok(_) => {
            reset_dialogue(dialogue).await?;
            bot.send_message(message.chat.id, texts::submission_queued())
                .reply_markup(student_keyboard())
                .await?;
        }
        Err(error) => send_app_error(bot, message.chat.id, error).await?,
    }

    Ok(())
}
