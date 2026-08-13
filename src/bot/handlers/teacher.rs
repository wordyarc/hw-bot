use chrono::{DateTime, Utc};
use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::MessageId;

use crate::app::{ChangeDeadline, CreateAssignment, GrantExtraAttempts};
use crate::domain::{AssignmentId, MAX_ATTEMPTS, TeachingCourseId, UserId};

use super::{Screen, current_assignment_screen, reset_dialogue};
use crate::bot::HandlerResult;
use crate::bot::assignment_files;
use crate::bot::callbacks::{
    AmountChoice, DeadlineAction, EXTRA_ATTEMPTS_PREFIX, MAX_ATTEMPTS_PREFIX, ScreenAction,
};
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotDialogue, BotState, DeadlineTarget, FlowState, ScreenState};
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{
    ask_input, assignment_actions_keyboard, assignment_status, assignment_students_keyboard,
    attempts_keyboard, calendar_keyboard, course_assignments_keyboard, courses_keyboard,
    deadline_left, deadline_options_keyboard, send_message_chunks, send_stale_button,
    teacher_keyboard, teacher_menu,
};

pub(super) async fn show_courses(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let courses = match services.courses.list_for_teacher(actor).await {
        Ok(courses) => courses,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    if courses.is_empty() {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, "📘 У вас пока нет учебных курсов.")
            .reply_markup(teacher_menu(None))
            .await?;
        return Ok(());
    }

    bot.send_message(chat_id, "📘 Выберите курс:")
        .reply_markup(courses_keyboard(&courses))
        .await?;
    reset_dialogue(dialogue).await
}

pub(super) async fn show_course_assignments(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        timezone,
    } = screen;
    let assignments = match services.assignments.list_for_course(actor, course_id).await {
        Ok(assignments) => assignments,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if assignments.is_empty() {
        bot.send_message(chat_id, "В этом курсе пока нет заданий.")
            .await?;
    } else {
        let mut text = String::from("Задания курса:\n");
        for assignment in &assignments {
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
        .reply_markup(course_assignments_keyboard(course_id, &assignments))
        .await?;
    reset_dialogue(dialogue).await
}

pub(super) async fn show_assignment(
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
        "{}\n\nСтатус: {}\n⏰ Дедлайн: {} ({})\n🔁 Попыток: {}{}\n\n📄 Текст задания:\n{}",
        assignment.title().as_str(),
        assignment_status(assignment.status()),
        timezone.format(assignment.deadline_at()),
        deadline_left(assignment.deadline_at(), Utc::now()),
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

pub(super) async fn show_students(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let students = match services
        .assignments
        .list_students_for_assignment(actor, assignment_id)
        .await
    {
        Ok(students) => students,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if students.is_empty() {
        bot.send_message(chat_id, "В группе пока нет студентов.")
            .await?;
    } else {
        let mut text = String::from("👥 Студенты группы:\n");
        for student in &students {
            let pending = if student.history.has_pending_submission() {
                " · ⏳ сдача на проверке"
            } else {
                ""
            };
            let accepted = if student.history.has_accepted_submission() {
                " · ✅ работа принята"
            } else {
                ""
            };
            text.push_str(&format!(
                "\n{} · использовано {} · дополнительных {}{}{}\n",
                student.student.display_name().as_str(),
                student.history.used_attempts(),
                student.history.extra_attempts(),
                pending,
                accepted,
            ));
        }
        send_message_chunks(bot, chat_id, text, None).await?;
    }

    let message = bot
        .send_message(chat_id, "Выберите студента:")
        .reply_markup(assignment_students_keyboard(assignment_id, &students))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingAttemptStudent {
            message_id: message.id,
            assignment_id,
            student_ids: students.iter().map(|item| item.student.id()).collect(),
        }))
        .await?;
    Ok(())
}

pub(super) async fn handle_screen_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: ScreenAction,
    state: BotState,
) -> HandlerResult {
    match action {
        ScreenAction::AddFile | ScreenAction::OpenFile(_) | ScreenAction::DeleteFile(_) => {
            unreachable!("assignment file actions are routed directly")
        }
        ScreenAction::FinishFileCreation => {
            let BotState::Screen(ScreenState::ManagingAssignmentFiles {
                message_id: current_message_id,
                assignment_id,
                creation_flow: true,
                ..
            }) = state
            else {
                return send_stale_button(screen.bot, screen.chat_id).await;
            };
            if current_message_id != message_id {
                return send_stale_button(screen.bot, screen.chat_id).await;
            }
            return show_assignment(screen, assignment_id).await;
        }
        ScreenAction::Publish(assignment_id)
        | ScreenAction::Archive(assignment_id)
        | ScreenAction::ChangeDeadline(assignment_id) => {
            if current_assignment_screen(assignment_id, message_id, &state).is_none() {
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

pub(super) async fn handle_max_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
        message_id: current_message_id,
        course_id,
        title,
        body,
    }) = state
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    if current_message_id != message_id {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }
    let Some(max_attempts) = chosen_amount(choice) else {
        return ask_input(
            screen.bot,
            screen.chat_id,
            &format!("Введите количество попыток от 1 до {MAX_ATTEMPTS}:"),
        )
        .await;
    };
    ask_deadline(
        screen.bot,
        screen.chat_id,
        DeadlineTarget::Create {
            course_id,
            title,
            body,
            max_attempts,
        },
        screen.dialogue,
        screen.timezone,
    )
    .await
}

pub(super) async fn select_student(
    screen: Screen<'_>,
    message_id: MessageId,
    student_id: UserId,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::ChoosingAttemptStudent {
        message_id: current_message_id,
        assignment_id,
        student_ids,
    }) = state
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    if current_message_id != message_id || !student_ids.contains(&student_id) {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }
    let sent = screen
        .bot
        .send_message(screen.chat_id, "Сколько дополнительных попыток выдать?")
        .reply_markup(attempts_keyboard(EXTRA_ATTEMPTS_PREFIX))
        .await?;
    screen
        .dialogue
        .update(BotState::Flow(FlowState::AwaitingAttemptAmount {
            message_id: sent.id,
            assignment_id,
            student_id,
        }))
        .await?;
    Ok(())
}

pub(super) async fn handle_extra_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::AwaitingAttemptAmount {
        message_id: current_message_id,
        assignment_id,
        student_id,
    }) = state
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    if current_message_id != message_id {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }
    let Some(amount) = chosen_amount(choice) else {
        return ask_input(
            screen.bot,
            screen.chat_id,
            &format!("Введите целое число от 1 до {MAX_ATTEMPTS}:"),
        )
        .await;
    };
    grant_attempts(screen, assignment_id, student_id, amount).await
}

fn chosen_amount(choice: AmountChoice) -> Option<u16> {
    match choice {
        AmountChoice::Preset(value) if value > 0 && value <= MAX_ATTEMPTS => Some(value),
        AmountChoice::Preset(_) | AmountChoice::Manual => None,
    }
}

pub(super) async fn handle_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    action: DeadlineAction,
    state: BotState,
) -> HandlerResult {
    let (current_message_id, target) = match state {
        BotState::Flow(FlowState::ChoosingDeadline { message_id, target })
        | BotState::Flow(FlowState::PickingDeadlineDate { message_id, target }) => {
            (message_id, target)
        }
        _ => return send_stale_button(screen.bot, screen.chat_id).await,
    };
    if current_message_id != message_id {
        return send_stale_button(screen.bot, screen.chat_id).await;
    }

    let date = match action {
        DeadlineAction::Days(days) => screen.timezone.in_days(u64::from(days)),
        DeadlineAction::Months(months) => screen.timezone.in_months(months),
        DeadlineAction::Pick(date) => Some(date),
        DeadlineAction::Manual => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingDeadlineText { target }))
                .await?;
            return ask_input(
                screen.bot,
                screen.chat_id,
                &format!(
                    "Введите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ ({}):",
                    screen.timezone.suffix()
                ),
            )
            .await;
        }
        DeadlineAction::Options => {
            let sent = screen
                .bot
                .send_message(screen.chat_id, deadline_prompt(&target, screen.timezone))
                .reply_markup(deadline_options_keyboard(screen.timezone))
                .await?;
            screen
                .dialogue
                .update(BotState::Flow(FlowState::ChoosingDeadline {
                    message_id: sent.id,
                    target,
                }))
                .await?;
            return Ok(());
        }
        DeadlineAction::Calendar => {
            let month = screen.timezone.today();
            screen
                .bot
                .edit_message_reply_markup(screen.chat_id, message_id)
                .reply_markup(calendar_keyboard(month, month))
                .await?;
            screen
                .dialogue
                .update(BotState::Flow(FlowState::PickingDeadlineDate {
                    message_id,
                    target,
                }))
                .await?;
            return Ok(());
        }
        DeadlineAction::Month(month) => {
            screen
                .bot
                .edit_message_reply_markup(screen.chat_id, message_id)
                .reply_markup(calendar_keyboard(month, screen.timezone.today()))
                .await?;
            screen
                .dialogue
                .update(BotState::Flow(FlowState::PickingDeadlineDate {
                    message_id,
                    target,
                }))
                .await?;
            return Ok(());
        }
    };

    let Some(deadline_at) = date.and_then(|date| screen.timezone.end_of(date)) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    apply_deadline(screen, target, deadline_at).await
}

fn deadline_prompt(target: &DeadlineTarget, timezone: &BotTimezone) -> String {
    let question = match target {
        DeadlineTarget::Create { .. } => "4/4. Когда истекает дедлайн?",
        DeadlineTarget::Change { .. } => "Новый дедлайн.",
    };
    format!(
        "{question} Время по умолчанию — {} ({}).",
        timezone.deadline_time().format("%H:%M"),
        timezone.suffix(),
    )
}

pub(super) async fn ask_max_attempts(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    course_id: TeachingCourseId,
    title: String,
    body: String,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, "3/4. Сколько попыток даёте студенту?")
        .reply_markup(attempts_keyboard(MAX_ATTEMPTS_PREFIX))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            course_id,
            title,
            body,
        }))
        .await?;
    Ok(())
}

pub(super) async fn ask_deadline(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    target: DeadlineTarget,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, deadline_prompt(&target, timezone))
        .reply_markup(deadline_options_keyboard(timezone))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingDeadline {
            message_id: sent.id,
            target,
        }))
        .await?;
    Ok(())
}

pub(super) async fn apply_deadline(
    screen: Screen<'_>,
    target: DeadlineTarget,
    deadline_at: DateTime<Utc>,
) -> HandlerResult {
    match target {
        DeadlineTarget::Create {
            course_id,
            title,
            body,
            max_attempts,
        } => {
            let result = screen
                .services
                .assignments
                .create(CreateAssignment {
                    teaching_course_id: course_id,
                    actor: screen.actor,
                    title,
                    body,
                    max_attempts,
                    created_at: Utc::now(),
                    deadline_at,
                })
                .await;
            match result {
                Ok(assignment) => {
                    screen
                        .bot
                        .send_message(
                            screen.chat_id,
                            format!(
                                "📝 Черновик создан. Дедлайн: {}",
                                screen.timezone.format(deadline_at)
                            ),
                        )
                        .reply_markup(teacher_keyboard())
                        .await?;
                    assignment_files::show(
                        screen.bot,
                        screen.chat_id,
                        screen.actor,
                        assignment.id(),
                        true,
                        screen.services,
                        screen.dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
            }
        }
        DeadlineTarget::Change { assignment_id } => match screen
            .services
            .assignments
            .change_deadline(ChangeDeadline {
                assignment_id,
                actor: screen.actor,
                deadline_at,
            })
            .await
        {
            Ok(_) => {
                screen
                    .bot
                    .send_message(
                        screen.chat_id,
                        format!(
                            "⏰ Дедлайн изменён: {}",
                            screen.timezone.format(deadline_at)
                        ),
                    )
                    .reply_markup(teacher_keyboard())
                    .await?;
                show_assignment(screen, assignment_id).await?;
            }
            Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
        },
    }
    Ok(())
}

pub(super) async fn grant_attempts(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    student_id: UserId,
    amount: u16,
) -> HandlerResult {
    match screen
        .services
        .assignments
        .grant_extra_attempts(GrantExtraAttempts {
            assignment_id,
            student_id,
            actor: screen.actor,
            amount,
        })
        .await
    {
        Ok(progress) => {
            screen
                .bot
                .send_message(
                    screen.chat_id,
                    format!(
                        "🎁 Добавлено попыток: {amount}. Всего дополнительных: {}.",
                        progress.extra_attempts()
                    ),
                )
                .reply_markup(teacher_keyboard())
                .await?;
            show_students(screen, assignment_id).await?;
        }
        Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
    }
    Ok(())
}
