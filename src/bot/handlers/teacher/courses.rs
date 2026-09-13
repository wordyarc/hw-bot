use crate::bot::callbacks::PageTarget;
use crate::bot::views::paginated_keyboard;
use crate::pagination::PageRequest;
use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::domain::{AssignmentId, TeachingCourseId};

use crate::bot::HandlerResult;
use crate::bot::context::Screen;
use crate::bot::handlers::reset_dialogue;
use crate::bot::state::{BotState, ScreenState};
use crate::bot::views::{
    assignment_actions_keyboard, assignment_status, course_assignments_keyboard, courses_keyboard,
    grading_lines, send_message_chunks, send_stale_button, teacher_menu,
};

pub(in crate::bot::handlers) async fn show_courses_page(
    screen: Screen<'_>,
    page: PageRequest,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        dialogue,
        ..
    } = screen;
    let services = &context.services;
    let courses = services.courses.list_for_teacher(actor, page).await?;
    if courses.items.is_empty() && page.page == 0 {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, "📘 У вас пока нет учебных курсов.")
            .reply_markup(teacher_menu(None))
            .await?;
        return Ok(());
    }

    bot.send_message(chat_id, "📘 Выберите курс:")
        .reply_markup(paginated_keyboard(
            courses_keyboard(&courses.items),
            PageTarget::Courses,
            courses.page,
            courses.next_page,
        ))
        .await?;
    reset_dialogue(dialogue).await
}

pub(in crate::bot::handlers) async fn show_course_assignments_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        dialogue,
        ..
    } = screen;
    let services = &context.services;
    let timezone = &context.timezone;
    let assignments = services
        .assignments
        .list_for_course(actor, course_id, page)
        .await?;

    if assignments.items.is_empty() {
        bot.send_message(chat_id, "В этом курсе пока нет заданий.")
            .await?;
    } else {
        let mut text = String::from("Задания курса:\n");
        for assignment in &assignments.items {
            text.push_str(&format!(
                "\n{} · {}\n⏰ {}\n",
                assignment.title().as_str(),
                assignment_status(assignment.status()),
                timezone.format(assignment.deadline_at()),
            ));
        }
        send_message_chunks(bot, chat_id, text, None).await?;
    }

    bot.send_message(chat_id, "Выберите задание или создайте новое:")
        .reply_markup(paginated_keyboard(
            course_assignments_keyboard(course_id, &assignments.items),
            PageTarget::Course(course_id),
            assignments.page,
            assignments.next_page,
        ))
        .await?;
    reset_dialogue(dialogue).await
}

pub(in crate::bot::handlers) async fn show_assignment(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        context,
        dialogue,
        ..
    } = screen;
    let services = &context.services;
    let timezone = &context.timezone;
    let assignment = services
        .assignments
        .find_for_teacher(actor, assignment_id)
        .await?;
    let published = assignment
        .published_at()
        .map(|published_at| format!("\n🚀 Опубликовано: {}", timezone.format(published_at)))
        .unwrap_or_default();
    let text = format!(
        "{}\n\nСтатус: {}{}\n🔁 Попыток: {}{}\n\n📄 Текст задания:\n{}",
        assignment.title().as_str(),
        assignment_status(assignment.status()),
        grading_lines(&assignment, Utc::now(), timezone),
        assignment.max_attempts(),
        published,
        assignment.body().as_str(),
    );
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, "Выберите действие:")
        .reply_markup(assignment_actions_keyboard(&assignment))
        .await?;
    dialogue
        .update(BotState::Screen(ScreenState::ManagingAssignment {
            message_id: message.id,
            assignment_id,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn publish_assignment(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if state
        .managing_assignment(message_id, assignment_id)
        .is_none()
    {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }
    screen
        .context
        .services
        .assignments
        .publish(assignment_id, screen.actor, Utc::now())
        .await?;
    screen
        .bot
        .send_message(screen.chat_id, "🚀 Задание опубликовано.")
        .await?;
    show_assignment(screen, assignment_id).await
}

pub(in crate::bot::handlers) async fn archive_assignment(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if state
        .managing_assignment(message_id, assignment_id)
        .is_none()
    {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }
    screen
        .context
        .services
        .assignments
        .archive(assignment_id, screen.actor)
        .await?;
    screen
        .bot
        .send_message(screen.chat_id, "📦 Задание архивировано.")
        .await?;
    show_assignment(screen, assignment_id).await
}

pub(in crate::bot::handlers) async fn ask_assignment_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    super::terms::begin_edit(screen, message_id, assignment_id, state).await
}
