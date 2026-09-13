use crate::bot::callbacks::PageTarget;
use crate::bot::views::paginated_keyboard;
use crate::pagination::PageRequest;
use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::domain::{AssignmentId, TeachingCourseId};

use crate::bot::HandlerResult;
use crate::bot::callbacks::ScreenAction;
use crate::bot::context::Screen;
use crate::bot::errors::send_app_error;
use crate::bot::handlers::reset_dialogue;
use crate::bot::state::{BotState, DeadlineTarget, FlowState, ScreenState};
use crate::bot::views::{
    assignment_actions_keyboard, assignment_status, confirm_invite_rotation_keyboard,
    course_assignments_keyboard, courses_keyboard, deadline_left, grading_lines,
    send_message_chunks, send_stale_button, teacher_menu,
};

use super::deadline::ask_deadline;

pub(in crate::bot::handlers) async fn show_courses(screen: Screen<'_>) -> HandlerResult {
    show_courses_page(screen, PageRequest::default()).await
}

pub(in crate::bot::handlers) async fn show_courses_page(
    screen: Screen<'_>,
    page: PageRequest,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let courses = match services.courses.list_for_teacher(actor, page).await {
        Ok(courses) => courses,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
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

pub(in crate::bot::handlers) async fn show_course_assignments(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    show_course_assignments_page(screen, course_id, PageRequest::default()).await
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
        services,
        dialogue,
        timezone,
        ..
    } = screen;
    let assignments = match services
        .assignments
        .list_for_course(actor, course_id, page)
        .await
    {
        Ok(assignments) => assignments,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

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
        services,
        dialogue,
        timezone,
        ..
    } = screen;
    let assignment = match services
        .assignments
        .find_for_teacher(actor, assignment_id)
        .await
    {
        Ok(assignment) => assignment,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let published = assignment
        .published_at()
        .map(|published_at| format!("\n🚀 Опубликовано: {}", timezone.format(published_at)))
        .unwrap_or_default();
    let text = format!(
        "{}\n\nСтатус: {}\n⏰ Дедлайн: {} ({}){}\n🔁 Попыток: {}{}\n\n📄 Текст задания:\n{}",
        assignment.title().as_str(),
        assignment_status(assignment.status()),
        timezone.format(assignment.deadline_at()),
        deadline_left(assignment.deadline_at(), Utc::now()),
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

pub(in crate::bot::handlers) async fn handle_screen_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: ScreenAction,
    state: BotState,
) -> HandlerResult {
    match action {
        ScreenAction::AddFile | ScreenAction::OpenFile(_) | ScreenAction::DeleteFile(_) => {
            unreachable!("assignment file actions are routed directly")
        }
        ScreenAction::RotateGroupInvite => {
            let Some((course_id, generation)) = state.group_invite(message_id) else {
                return send_stale_button(screen.bot, screen.chat_id).await;
            };

            let sent = screen
                .bot
                .send_message(
                    screen.chat_id,
                    "Перевыпустить ссылку? Прежняя сразу перестанет работать — в том числе у \
                     других курсов этой же группы. Уже зарегистрированных студентов это не \
                     затрагивает.",
                )
                .reply_markup(confirm_invite_rotation_keyboard())
                .await?;

            screen
                .dialogue
                .update(BotState::Flow(FlowState::ConfirmingInviteRotation {
                    message_id: sent.id,
                    course_id,
                    generation,
                }))
                .await?;
            return Ok(());
        }
        ScreenAction::FinishFileCreation => {
            // Кнопка «готово» есть только во флоу создания: вне его она из старого сообщения.
            let Some((assignment_id, _, true)) = state.managing_assignment_files(message_id) else {
                return send_stale_button(screen.bot, screen.chat_id).await;
            };
            return show_assignment(screen, assignment_id).await;
        }
        ScreenAction::Publish(assignment_id)
        | ScreenAction::Archive(assignment_id)
        | ScreenAction::ChangeDeadline(assignment_id) => {
            if state
                .managing_assignment(message_id, assignment_id)
                .is_none()
            {
                return send_stale_button(screen.bot, screen.chat_id).await;
            }
        }
    }

    let assignment_id = match action {
        ScreenAction::Publish(id)
        | ScreenAction::Archive(id)
        | ScreenAction::ChangeDeadline(id) => id,
        _ => unreachable!(),
    };
    match action {
        ScreenAction::Publish(_) => match screen
            .services
            .assignments
            .publish(assignment_id, screen.actor, Utc::now())
            .await
        {
            Ok(_) => {
                screen
                    .bot
                    .send_message(screen.chat_id, "🚀 Задание опубликовано.")
                    .await?;
                show_assignment(screen, assignment_id).await?;
            }
            Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
        },
        ScreenAction::Archive(_) => match screen
            .services
            .assignments
            .archive(assignment_id, screen.actor)
            .await
        {
            Ok(_) => {
                screen
                    .bot
                    .send_message(screen.chat_id, "📦 Задание архивировано.")
                    .await?;
                show_assignment(screen, assignment_id).await?;
            }
            Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
        },
        ScreenAction::ChangeDeadline(_) => {
            ask_deadline(
                screen.bot,
                screen.chat_id,
                DeadlineTarget::Change { assignment_id },
                screen.dialogue,
                screen.timezone,
            )
            .await?;
        }
        _ => unreachable!(),
    }
    Ok(())
}
