use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDateTime, Utc};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::{Bot, Requester};
use teloxide::types::{
    CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message, MessageId, User,
};
use teloxide::utils::command::BotCommands;
use uuid::Uuid;

use crate::app::{
    Actor, ChangeDeadline, CreateAssignment, ErrorCode, GrantExtraAttempts, RegisterStudent,
    ReviewSubmission, SubmitHomework,
};
use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, AssignmentId, AssignmentStatus, DISPLAY_NAME_MAX_LEN, MAX_ATTEMPTS,
    ReviewDecision, Role, SubmissionId, TeachingCourseId, TelegramUserId, USER_TEXT_MAX_LEN,
    UserId,
};

use super::errors::send_app_error;
use super::state::{BotDialogue, BotState};
use super::views::{
    assignment_actions_keyboard, assignment_status, assignment_students_keyboard, button_label,
    course_assignments_keyboard, courses_keyboard, format_deadline, group_keyboard,
    review_keyboard, send_message_chunks, send_stale_button, student_menu, submission_status,
    teacher_menu,
};
use super::{BotServices, HandlerResult};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub(super) enum Command {
    #[command(description = "открыть главное меню")]
    Start,
    #[command(description = "открыть главное меню")]
    Menu,
    #[command(description = "отменить текущее действие")]
    Cancel,
    #[command(description = "показать помощь")]
    Help,
}

pub(super) async fn handle_command(
    bot: Bot,
    message: Message,
    command: Command,
    dialogue: BotDialogue,
    services: BotServices,
) -> HandlerResult {
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    match command {
        Command::Start | Command::Menu => {
            dialogue.exit().await?;
            show_home(
                &bot,
                message.chat.id,
                actor_from_message(&message)?,
                &services,
                &dialogue,
            )
            .await?;
        }
        Command::Cancel => {
            dialogue.exit().await?;
            bot.send_message(message.chat.id, "Текущее действие отменено.")
                .await?;
            show_home(
                &bot,
                message.chat.id,
                actor_from_message(&message)?,
                &services,
                &dialogue,
            )
            .await?;
        }
        Command::Help => {
            bot.send_message(message.chat.id, Command::descriptions().to_string())
                .await?;
        }
    }

    Ok(())
}

pub(super) async fn handle_message(
    bot: Bot,
    message: Message,
    dialogue: BotDialogue,
    services: BotServices,
) -> HandlerResult {
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    let Some(text) = message.text() else {
        bot.send_message(message.chat.id, "Пришлите текстовое сообщение.")
            .await?;
        return Ok(());
    };
    let actor = actor_from_message(&message)?;

    match dialogue.get().await?.unwrap_or_default() {
        BotState::Idle => {
            bot.send_message(
                message.chat.id,
                "Используйте кнопки меню или команду /menu.",
            )
            .await?;
        }
        BotState::AwaitingRegistrationName => {
            if text.trim().is_empty() {
                bot.send_message(message.chat.id, "Имя не должно быть пустым.")
                    .await?;
                return Ok(());
            }
            if !ensure_text_limit(&bot, message.chat.id, text, DISPLAY_NAME_MAX_LEN, "Имя").await?
            {
                return Ok(());
            }
            let groups = match services.users.list_student_groups().await {
                Ok(groups) => groups,
                Err(error) => {
                    send_app_error(&bot, message.chat.id, error).await?;
                    return Ok(());
                }
            };
            if groups.is_empty() {
                bot.send_message(
                    message.chat.id,
                    "Пока нет доступных учебных групп. Обратитесь к администратору.",
                )
                .await?;
                return Ok(());
            }

            let group_codes: Vec<_> = groups
                .iter()
                .map(|group| group.code().as_str().to_owned())
                .collect();
            dialogue
                .update(BotState::AwaitingRegistrationGroup {
                    display_name: text.trim().to_owned(),
                    group_codes: group_codes.clone(),
                })
                .await?;
            bot.send_message(message.chat.id, "Выберите учебную группу:")
                .reply_markup(group_keyboard(&group_codes))
                .await?;
        }
        BotState::AwaitingRegistrationGroup { .. } => {
            bot.send_message(message.chat.id, "Выберите группу кнопкой выше.")
                .await?;
        }
        BotState::AwaitingSubmission { assignment_id } => {
            if !ensure_text_limit(
                &bot,
                message.chat.id,
                text,
                USER_TEXT_MAX_LEN,
                "Текст работы",
            )
            .await?
            {
                return Ok(());
            }
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
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
                    dialogue.exit().await?;
                    bot.send_message(message.chat.id, "Работа отправлена на проверку.")
                        .reply_markup(student_menu())
                        .await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::AwaitingReviewDecision { .. } => {
            bot.send_message(message.chat.id, "Выберите решение кнопкой под сдачей.")
                .await?;
        }
        BotState::AwaitingReviewComment {
            submission_id,
            decision,
        } => {
            if !ensure_text_limit(
                &bot,
                message.chat.id,
                text,
                USER_TEXT_MAX_LEN,
                "Комментарий",
            )
            .await?
            {
                return Ok(());
            }
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
                return Ok(());
            };
            let result = services
                .reviews
                .review_submission(ReviewSubmission {
                    submission_id,
                    actor,
                    decision,
                    comment: text.to_owned(),
                    reviewed_at: Utc::now(),
                })
                .await;

            match result {
                Ok(_) => {
                    dialogue.exit().await?;
                    bot.send_message(
                        message.chat.id,
                        "Решение сохранено, студент получит уведомление.",
                    )
                    .reply_markup(teacher_menu(None))
                    .await?;
                }
                Err(error) if error.code() == ErrorCode::SubmissionAlreadyReviewed => {
                    dialogue.exit().await?;
                    send_app_error(&bot, message.chat.id, error).await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::ChoosingCourse { .. } => {
            bot.send_message(message.chat.id, "Выберите курс кнопкой выше.")
                .await?;
        }
        BotState::ChoosingAssignment { .. } => {
            bot.send_message(
                message.chat.id,
                "Выберите задание или действие кнопкой выше.",
            )
            .await?;
        }
        BotState::ManagingAssignment { .. } => {
            bot.send_message(message.chat.id, "Выберите действие кнопкой под заданием.")
                .await?;
        }
        BotState::AwaitingAssignmentTitle { course_id } => {
            if text.trim().is_empty() {
                bot.send_message(message.chat.id, "Название не должно быть пустым.")
                    .await?;
                return Ok(());
            }
            if !ensure_text_limit(
                &bot,
                message.chat.id,
                text,
                ASSIGNMENT_TITLE_MAX_LEN,
                "Название",
            )
            .await?
            {
                return Ok(());
            }
            dialogue
                .update(BotState::AwaitingAssignmentBody {
                    course_id,
                    title: text.to_owned(),
                })
                .await?;
            bot.send_message(message.chat.id, "Пришлите текст задания одним сообщением:")
                .await?;
        }
        BotState::AwaitingAssignmentBody { course_id, title } => {
            if text.trim().is_empty() {
                bot.send_message(message.chat.id, "Текст задания не должен быть пустым.")
                    .await?;
                return Ok(());
            }
            if !ensure_text_limit(
                &bot,
                message.chat.id,
                text,
                USER_TEXT_MAX_LEN,
                "Текст задания",
            )
            .await?
            {
                return Ok(());
            }
            dialogue
                .update(BotState::AwaitingAssignmentMaxAttempts {
                    course_id,
                    title,
                    body: text.to_owned(),
                })
                .await?;
            bot.send_message(
                message.chat.id,
                format!("Введите количество попыток от 1 до {MAX_ATTEMPTS}:"),
            )
            .await?;
        }
        BotState::AwaitingAssignmentMaxAttempts {
            course_id,
            title,
            body,
        } => {
            let Some(max_attempts) = parse_positive_attempts(text) else {
                bot.send_message(
                    message.chat.id,
                    format!("Введите целое число от 1 до {MAX_ATTEMPTS}."),
                )
                .await?;
                return Ok(());
            };
            dialogue
                .update(BotState::AwaitingAssignmentDeadline {
                    course_id,
                    title,
                    body,
                    max_attempts,
                })
                .await?;
            bot.send_message(
                message.chat.id,
                "Введите дедлайн в UTC в формате ДД.ММ.ГГГГ ЧЧ:ММ:",
            )
            .await?;
        }
        BotState::AwaitingAssignmentDeadline {
            course_id,
            title,
            body,
            max_attempts,
        } => {
            let Some(deadline_at) = parse_deadline(text) else {
                send_deadline_format_error(&bot, message.chat.id).await?;
                return Ok(());
            };
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
                return Ok(());
            };
            let result = services
                .assignments
                .create(CreateAssignment {
                    teaching_course_id: course_id,
                    actor,
                    title,
                    body,
                    max_attempts,
                    created_at: Utc::now(),
                    deadline_at,
                })
                .await;

            match result {
                Ok(assignment) => {
                    bot.send_message(message.chat.id, "Черновик создан.")
                        .await?;
                    show_teacher_assignment(
                        &bot,
                        message.chat.id,
                        actor,
                        assignment.id(),
                        &services,
                        &dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::AwaitingDeadlineChange { assignment_id } => {
            let Some(deadline_at) = parse_deadline(text) else {
                send_deadline_format_error(&bot, message.chat.id).await?;
                return Ok(());
            };
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
                return Ok(());
            };
            match services
                .assignments
                .change_deadline(ChangeDeadline {
                    assignment_id,
                    actor,
                    deadline_at,
                })
                .await
            {
                Ok(_) => {
                    bot.send_message(message.chat.id, "Дедлайн изменён.")
                        .await?;
                    show_teacher_assignment(
                        &bot,
                        message.chat.id,
                        actor,
                        assignment_id,
                        &services,
                        &dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::ChoosingAttemptStudent { .. } => {
            bot.send_message(message.chat.id, "Выберите студента кнопкой выше.")
                .await?;
        }
        BotState::AwaitingAttemptAmount {
            assignment_id,
            student_id,
        } => {
            let Some(amount) = parse_positive_attempts(text) else {
                bot.send_message(
                    message.chat.id,
                    format!("Введите целое число от 1 до {MAX_ATTEMPTS}."),
                )
                .await?;
                return Ok(());
            };
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
                return Ok(());
            };
            match services
                .assignments
                .grant_extra_attempts(GrantExtraAttempts {
                    assignment_id,
                    student_id,
                    actor,
                    amount,
                })
                .await
            {
                Ok(progress) => {
                    bot.send_message(
                        message.chat.id,
                        format!(
                            "Добавлено попыток: {amount}. Всего дополнительных: {}.",
                            progress.extra_attempts()
                        ),
                    )
                    .await?;
                    show_assignment_students(
                        &bot,
                        message.chat.id,
                        actor,
                        assignment_id,
                        &services,
                        &dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
    }

    Ok(())
}

async fn ensure_text_limit(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    max_len: usize,
    field: &str,
) -> HandlerResult<bool> {
    if text.trim().chars().count() <= max_len {
        return Ok(true);
    }

    bot.send_message(
        chat_id,
        format!("{field} не должно быть длиннее {max_len} символов."),
    )
    .await?;
    Ok(false)
}

pub(super) async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    dialogue: BotDialogue,
    services: BotServices,
) -> HandlerResult {
    bot.answer_callback_query(query.id.clone())
        .cache_time(0)
        .await?;

    let Some(message) = query.regular_message() else {
        return Ok(());
    };
    if !message.chat.is_private() {
        return Ok(());
    }
    let chat_id = message.chat.id;
    let telegram_user_id = actor_from_user(&query.from)?;
    let Some(data) = query.data.as_deref() else {
        return Ok(());
    };
    let state = dialogue.get().await?.unwrap_or_default();

    if let Some(index) = data.strip_prefix("register_group:") {
        let BotState::AwaitingRegistrationGroup {
            display_name,
            group_codes,
        } = state
        else {
            send_stale_button(&bot, chat_id).await?;
            return Ok(());
        };
        let Some(group_code) = index
            .parse::<usize>()
            .ok()
            .and_then(|index| group_codes.get(index))
        else {
            send_stale_button(&bot, chat_id).await?;
            return Ok(());
        };

        match services
            .users
            .register_student(RegisterStudent {
                telegram_user_id,
                display_name,
                student_group_code: group_code.clone(),
                registered_at: Utc::now(),
            })
            .await
        {
            Ok(_) => {
                dialogue.exit().await?;
                bot.send_message(chat_id, "Регистрация завершена.")
                    .reply_markup(student_menu())
                    .await?;
            }
            Err(error) => send_app_error(&bot, chat_id, error).await?,
        }
        return Ok(());
    }

    let Some(actor) = resolve_actor(&bot, chat_id, telegram_user_id, &services).await? else {
        return Ok(());
    };

    match data {
        "menu:assignments" => show_student_assignments(&bot, chat_id, actor, &services).await?,
        "menu:courses" => show_courses(&bot, chat_id, actor, &services, &dialogue).await?,
        "review:next" => show_next_review(&bot, chat_id, actor, &services, &dialogue).await?,
        _ if data.starts_with("review:accept:") || data.starts_with("review:reject:") => {
            let Some((decision, callback_submission_id)) = parse_review_callback(data) else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            let BotState::AwaitingReviewDecision { submission_id } = state else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if submission_id != callback_submission_id {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            dialogue
                .update(BotState::AwaitingReviewComment {
                    submission_id,
                    decision,
                })
                .await?;
            bot.send_message(chat_id, "Напишите обязательный комментарий к решению:")
                .await?;
        }
        _ if data.starts_with("course:") => {
            let Some(id) = parse_uuid_callback(data, "course:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            let course_id = TeachingCourseId::from_uuid(id);
            let valid = match &state {
                BotState::ChoosingCourse {
                    message_id,
                    course_ids,
                } => *message_id == message.id && course_ids.contains(&course_id),
                BotState::ManagingAssignment {
                    message_id,
                    course_id: current_course_id,
                    ..
                } => *message_id == message.id && *current_course_id == course_id,
                _ => false,
            };
            if !valid {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            show_course_assignments(&bot, chat_id, actor, course_id, &services, &dialogue).await?;
        }
        _ if data.starts_with("manage:") => {
            let Some(id) = parse_uuid_callback(data, "manage:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            let assignment_id = AssignmentId::from_uuid(id);
            let valid = match &state {
                BotState::ChoosingAssignment {
                    message_id,
                    assignment_ids,
                    ..
                } => *message_id == message.id && assignment_ids.contains(&assignment_id),
                BotState::ChoosingAttemptStudent {
                    message_id,
                    assignment_id: current_assignment_id,
                    ..
                } => *message_id == message.id && *current_assignment_id == assignment_id,
                _ => false,
            };
            if !valid {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            show_teacher_assignment(&bot, chat_id, actor, assignment_id, &services, &dialogue)
                .await?;
        }
        _ if data.starts_with("create:") => {
            let Some(id) = parse_uuid_callback(data, "create:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            let course_id = TeachingCourseId::from_uuid(id);
            let BotState::ChoosingAssignment {
                message_id,
                course_id: current_course_id,
                ..
            } = state
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if message_id != message.id || current_course_id != course_id {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            dialogue
                .update(BotState::AwaitingAssignmentTitle { course_id })
                .await?;
            bot.send_message(chat_id, "Введите название задания:")
                .await?;
        }
        _ if data.starts_with("publish:") => {
            let Some(assignment_id) =
                managed_assignment_callback(data, "publish:", message.id, &state)
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            match services
                .assignments
                .publish(assignment_id, actor, Utc::now())
                .await
            {
                Ok(_) => {
                    bot.send_message(chat_id, "Задание опубликовано.").await?;
                    show_teacher_assignment(
                        &bot,
                        chat_id,
                        actor,
                        assignment_id,
                        &services,
                        &dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(&bot, chat_id, error).await?,
            }
        }
        _ if data.starts_with("archive:") => {
            let Some(assignment_id) =
                managed_assignment_callback(data, "archive:", message.id, &state)
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            match services.assignments.archive(assignment_id, actor).await {
                Ok(_) => {
                    bot.send_message(chat_id, "Задание архивировано.").await?;
                    show_teacher_assignment(
                        &bot,
                        chat_id,
                        actor,
                        assignment_id,
                        &services,
                        &dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(&bot, chat_id, error).await?,
            }
        }
        _ if data.starts_with("deadline:") => {
            let Some(assignment_id) =
                managed_assignment_callback(data, "deadline:", message.id, &state)
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            dialogue
                .update(BotState::AwaitingDeadlineChange { assignment_id })
                .await?;
            bot.send_message(
                chat_id,
                "Введите новый дедлайн в UTC в формате ДД.ММ.ГГГГ ЧЧ:ММ:",
            )
            .await?;
        }
        _ if data.starts_with("attempts:") => {
            let Some(assignment_id) =
                managed_assignment_callback(data, "attempts:", message.id, &state)
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_assignment_students(&bot, chat_id, actor, assignment_id, &services, &dialogue)
                .await?;
        }
        _ if data.starts_with("student:") => {
            let Some(id) = parse_uuid_callback(data, "student:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            let student_id = UserId::from_uuid(id);
            let BotState::ChoosingAttemptStudent {
                message_id,
                assignment_id,
                student_ids,
            } = state
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if message_id != message.id || !student_ids.contains(&student_id) {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            dialogue
                .update(BotState::AwaitingAttemptAmount {
                    assignment_id,
                    student_id,
                })
                .await?;
            bot.send_message(chat_id, "Сколько дополнительных попыток выдать?")
                .await?;
        }
        _ if data.starts_with("assignment:") => {
            let Some(id) = parse_uuid_callback(data, "assignment:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_assignment(&bot, chat_id, actor, AssignmentId::from_uuid(id), &services).await?;
        }
        _ if data.starts_with("submit:") => {
            let Some(id) = parse_uuid_callback(data, "submit:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            dialogue
                .update(BotState::AwaitingSubmission {
                    assignment_id: AssignmentId::from_uuid(id),
                })
                .await?;
            bot.send_message(chat_id, "Пришлите ссылку или текст сдачи одним сообщением:")
                .await?;
        }
        _ => send_stale_button(&bot, chat_id).await?,
    }

    Ok(())
}

async fn show_home(
    bot: &Bot,
    chat_id: ChatId,
    actor: TelegramUserId,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
    match services.users.find_by_telegram_user_id(actor).await {
        Ok(registered) => match registered.user.role() {
            Role::Student { .. } => {
                bot.send_message(
                    chat_id,
                    format!("Здравствуйте, {}.", registered.user.display_name().as_str()),
                )
                .reply_markup(student_menu())
                .await?;
            }
            Role::Teacher | Role::Admin => {
                let actor = Actor::new(registered.user.id());
                let pending = match services.reviews.queue_size(actor).await {
                    Ok(pending) => pending,
                    Err(error) => {
                        send_app_error(bot, chat_id, error).await?;
                        return Ok(());
                    }
                };
                bot.send_message(
                    chat_id,
                    format!("Здравствуйте, {}.", registered.user.display_name().as_str()),
                )
                .reply_markup(teacher_menu(Some(pending)))
                .await?;
            }
        },
        Err(error) if error.code() == ErrorCode::NotFound => {
            dialogue.update(BotState::AwaitingRegistrationName).await?;
            bot.send_message(chat_id, "Вы ещё не зарегистрированы. Как вас называть?")
                .await?;
        }
        Err(error) => send_app_error(bot, chat_id, error).await?,
    }

    Ok(())
}

async fn show_student_assignments(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    services: &BotServices,
) -> HandlerResult {
    let assignments = match services.assignments.list_for_student(actor).await {
        Ok(assignments) => assignments,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };

    if assignments.is_empty() {
        bot.send_message(chat_id, "Опубликованных заданий пока нет.")
            .reply_markup(student_menu())
            .await?;
        return Ok(());
    }

    let mut text = String::from("Ваши задания:\n");
    let mut rows = Vec::with_capacity(assignments.len());
    for item in assignments {
        let assignment = &item.assignment;
        let remaining = item.history.remaining_attempts_for(assignment);
        text.push_str(&format!(
            "\n{}\nдо {} · осталось попыток: {}\n",
            assignment.title().as_str(),
            format_deadline(assignment.deadline_at()),
            remaining,
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            button_label(assignment.title().as_str()),
            format!("assignment:{}", assignment.id().as_uuid()),
        )]);
    }

    send_message_chunks(bot, chat_id, text, Some(InlineKeyboardMarkup::new(rows))).await?;
    Ok(())
}

async fn show_assignment(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    assignment_id: AssignmentId,
    services: &BotServices,
) -> HandlerResult {
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
    let summary = format!(
        "{}\n\nДедлайн: {}\nПопытки: использовано {}, доступно {}, осталось {}",
        assignment.title().as_str(),
        format_deadline(assignment.deadline_at()),
        history.used_attempts(),
        history.allowed_attempts_for(assignment),
        history.remaining_attempts_for(assignment),
    );

    let mut rows = Vec::new();
    if assignment.status() == AssignmentStatus::Published {
        rows.push(vec![InlineKeyboardButton::callback(
            "Сдать работу",
            format!("submit:{}", assignment.id().as_uuid()),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "К списку заданий",
        "menu:assignments",
    )]);

    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, "Текст задания:").await?;
    bot.send_message(chat_id, assignment.body().as_str())
        .await?;
    for submission in &details.submissions {
        bot.send_message(
            chat_id,
            format!(
                "Попытка {}: {}",
                submission.attempt_number(),
                submission_status(submission.status()),
            ),
        )
        .await?;
        bot.send_message(chat_id, submission.content().as_str())
            .await?;
        if let Some(comment) = submission.review_comment() {
            bot.send_message(chat_id, "Комментарий преподавателя:")
                .await?;
            bot.send_message(chat_id, comment.as_str()).await?;
        }
    }
    bot.send_message(chat_id, "Выберите действие:")
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

async fn show_courses(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let courses = match services.courses.list_for_teacher(actor).await {
        Ok(courses) => courses,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    if courses.is_empty() {
        dialogue.exit().await?;
        bot.send_message(chat_id, "У вас пока нет учебных курсов.")
            .reply_markup(teacher_menu(None))
            .await?;
        return Ok(());
    }

    let mut text = String::from("Ваши курсы:\n");
    for course in &courses {
        text.push_str(&format!(
            "\n{} · {}\n",
            course.subject.name().as_str(),
            course.group.code().as_str(),
        ));
    }
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, "Выберите курс:")
        .reply_markup(courses_keyboard(&courses))
        .await?;
    dialogue
        .update(BotState::ChoosingCourse {
            message_id: message.id,
            course_ids: courses.iter().map(|course| course.course.id()).collect(),
        })
        .await?;
    Ok(())
}

async fn show_course_assignments(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    course_id: TeachingCourseId,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
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
                "\n{} · {}\nдедлайн: {}\n",
                assignment.title().as_str(),
                assignment_status(assignment.status()),
                format_deadline(assignment.deadline_at()),
            ));
        }
        send_message_chunks(bot, chat_id, text, None).await?;
    }

    let message = bot
        .send_message(chat_id, "Выберите задание или создайте черновик:")
        .reply_markup(course_assignments_keyboard(course_id, &assignments))
        .await?;
    dialogue
        .update(BotState::ChoosingAssignment {
            message_id: message.id,
            course_id,
            assignment_ids: assignments
                .iter()
                .map(|assignment| assignment.id())
                .collect(),
        })
        .await?;
    Ok(())
}

async fn show_teacher_assignment(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    assignment_id: AssignmentId,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
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
        .map(|published_at| format!("\nОпубликовано: {}", format_deadline(published_at)))
        .unwrap_or_default();
    let text = format!(
        "{}\n\nСтатус: {}\nДедлайн: {}\nПопыток: {}{}\n\nТекст задания:\n{}",
        assignment.title().as_str(),
        assignment_status(assignment.status()),
        format_deadline(assignment.deadline_at()),
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
        .update(BotState::ManagingAssignment {
            message_id: message.id,
            course_id: assignment.teaching_course_id(),
            assignment_id,
        })
        .await?;
    Ok(())
}

async fn show_assignment_students(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    assignment_id: AssignmentId,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
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
        let mut text = String::from("Студенты группы:\n");
        for student in &students {
            let pending = if student.history.has_pending_submission() {
                " · сдача на проверке"
            } else {
                ""
            };
            let accepted = if student.history.has_accepted_submission() {
                " · работа принята"
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
        .update(BotState::ChoosingAttemptStudent {
            message_id: message.id,
            assignment_id,
            student_ids: students.iter().map(|item| item.student.id()).collect(),
        })
        .await?;
    Ok(())
}

async fn show_next_review(
    bot: &Bot,
    chat_id: ChatId,
    actor: Actor,
    services: &BotServices,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let item = match services.reviews.take_next_from_queue(actor).await {
        Ok(item) => item,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let Some(item) = item else {
        dialogue.exit().await?;
        bot.send_message(chat_id, "Очередь проверки пуста.")
            .reply_markup(teacher_menu(Some(0)))
            .await?;
        return Ok(());
    };

    dialogue
        .update(BotState::AwaitingReviewDecision {
            submission_id: item.submission.id(),
        })
        .await?;
    let summary = format!(
        "{}\nСтудент: {}",
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
    );
    let submission_id = item.submission.id();
    let keyboard = review_keyboard(submission_id);
    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, "Текст задания:").await?;
    bot.send_message(chat_id, item.assignment.body().as_str())
        .await?;
    bot.send_message(chat_id, "Сдача студента:").await?;
    bot.send_message(chat_id, item.submission.content().as_str())
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

async fn ensure_private_message(bot: &Bot, message: &Message) -> HandlerResult<bool> {
    if message.chat.is_private() {
        return Ok(true);
    }

    bot.send_message(message.chat.id, "Бот работает только в личных сообщениях.")
        .await?;
    Ok(false)
}

fn actor_from_message(message: &Message) -> HandlerResult<TelegramUserId> {
    actor_from_user(
        message
            .from
            .as_ref()
            .context("private Telegram message has no sender")?,
    )
}

fn actor_from_user(user: &User) -> HandlerResult<TelegramUserId> {
    let value = i64::try_from(user.id.0).context("Telegram user id does not fit into i64")?;
    TelegramUserId::new(value).map_err(|error| anyhow!(error))
}

fn parse_uuid_callback(data: &str, prefix: &str) -> Option<Uuid> {
    data.strip_prefix(prefix)?.parse().ok()
}

fn parse_review_callback(data: &str) -> Option<(ReviewDecision, SubmissionId)> {
    let (decision, id) = data
        .strip_prefix("review:accept:")
        .map(|id| (ReviewDecision::Accept, id))
        .or_else(|| {
            data.strip_prefix("review:reject:")
                .map(|id| (ReviewDecision::Reject, id))
        })?;

    Some((decision, SubmissionId::from_uuid(id.parse().ok()?)))
}

fn managed_assignment_callback(
    data: &str,
    prefix: &str,
    message_id: MessageId,
    state: &BotState,
) -> Option<AssignmentId> {
    let assignment_id = AssignmentId::from_uuid(parse_uuid_callback(data, prefix)?);
    let BotState::ManagingAssignment {
        message_id: current_message_id,
        assignment_id: current_assignment_id,
        ..
    } = state
    else {
        return None;
    };

    (*current_message_id == message_id && *current_assignment_id == assignment_id)
        .then_some(assignment_id)
}

fn parse_deadline(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    let value = NaiveDateTime::parse_from_str(text, "%d.%m.%Y %H:%M").ok()?;
    (value.format("%d.%m.%Y %H:%M").to_string() == text).then(|| value.and_utc())
}

fn parse_positive_attempts(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: u16 = text.parse().ok()?;

    (value > 0 && value <= MAX_ATTEMPTS).then_some(value)
}

async fn send_deadline_format_error(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(
        chat_id,
        "Не удалось разобрать дату. Используйте формат ДД.ММ.ГГГГ ЧЧ:ММ в UTC.",
    )
    .await?;
    Ok(())
}

async fn resolve_actor(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: TelegramUserId,
    services: &BotServices,
) -> HandlerResult<Option<Actor>> {
    match services.users.resolve_actor(telegram_user_id).await {
        Ok(actor) => Ok(Some(actor)),
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::app::ApplicationContainer;

    use super::*;

    #[test]
    fn callback_uuid_parser_rejects_wrong_prefix_and_invalid_uuid() {
        let id = Uuid::now_v7();

        assert_eq!(
            parse_uuid_callback(&format!("assignment:{id}"), "assignment:"),
            Some(id)
        );
        assert_eq!(
            parse_uuid_callback(&format!("submit:{id}"), "assignment:"),
            None
        );
        assert_eq!(parse_uuid_callback("assignment:nope", "assignment:"), None);
        assert_eq!(
            parse_uuid_callback(&format!("submit:{id}"), "submit:"),
            Some(id)
        );
    }

    #[test]
    fn cancel_command_remains_part_of_the_telegram_contract() {
        assert!(matches!(
            Command::parse("/cancel", "test_bot").unwrap(),
            Command::Cancel
        ));
    }

    #[test]
    fn review_callbacks_carry_the_submission_the_buttons_belong_to() {
        let submission_id = SubmissionId::new();
        let keyboard = review_keyboard(submission_id);
        let json = serde_json::to_value(keyboard).unwrap();
        let accept = json["inline_keyboard"][0][0]["callback_data"]
            .as_str()
            .unwrap();
        let reject = json["inline_keyboard"][0][1]["callback_data"]
            .as_str()
            .unwrap();

        assert_eq!(
            parse_review_callback(accept),
            Some((ReviewDecision::Accept, submission_id))
        );
        assert_eq!(
            parse_review_callback(reject),
            Some((ReviewDecision::Reject, submission_id))
        );
        assert!(accept.len() <= 64);
        assert!(reject.len() <= 64);
    }

    #[test]
    fn old_review_callback_identifies_a_different_submission() {
        let current = SubmissionId::new();
        let old = SubmissionId::new();
        let (_, callback_submission) =
            parse_review_callback(&format!("review:accept:{}", old.as_uuid())).unwrap();

        assert_ne!(callback_submission, current);
    }

    #[test]
    fn teacher_inputs_use_strict_formats() {
        assert_eq!(
            parse_deadline("31.12.2030 23:59").unwrap(),
            "2030-12-31T23:59:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert!(parse_deadline("31.12.2030 23:59 UTC").is_none());
        assert!(parse_deadline("1.12.2030 23:59").is_none());
        assert!(parse_deadline("31.02.2030 23:59").is_none());

        assert_eq!(parse_positive_attempts("1"), Some(1));
        assert_eq!(
            parse_positive_attempts(&MAX_ATTEMPTS.to_string()),
            Some(MAX_ATTEMPTS)
        );
        assert!(parse_positive_attempts("0").is_none());
        assert!(parse_positive_attempts("+1").is_none());
        assert!(parse_positive_attempts("32768").is_none());
    }

    #[test]
    fn assignment_action_requires_current_keyboard_and_assignment() {
        let current = AssignmentId::new();
        let other = AssignmentId::new();
        let state = BotState::ManagingAssignment {
            message_id: MessageId(10),
            course_id: TeachingCourseId::new(),
            assignment_id: current,
        };

        assert_eq!(
            managed_assignment_callback(
                &format!("archive:{}", current.as_uuid()),
                "archive:",
                MessageId(10),
                &state,
            ),
            Some(current)
        );
        assert!(
            managed_assignment_callback(
                &format!("archive:{}", current.as_uuid()),
                "archive:",
                MessageId(11),
                &state,
            )
            .is_none()
        );
        assert!(
            managed_assignment_callback(
                &format!("archive:{}", other.as_uuid()),
                "archive:",
                MessageId(10),
                &state,
            )
            .is_none()
        );
    }

    #[tokio::test]
    async fn help_command_sends_message_through_teloxide() {
        let telegram = MockServer::start().await;
        let descriptions = Command::descriptions().to_string();
        Mock::given(method("POST"))
            .and(path("/bottest-token/SendMessage"))
            .and(body_json(json!({
                "chat_id": 42,
                "text": descriptions,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {
                    "message_id": 2,
                    "date": 0,
                    "chat": { "id": 42, "type": "private" },
                    "text": descriptions,
                }
            })))
            .expect(1)
            .mount(&telegram)
            .await;
        let bot = Bot::new("test-token").set_api_url(telegram.uri().parse().unwrap());
        let message: Message = serde_json::from_value(json!({
            "message_id": 1,
            "date": 0,
            "chat": { "id": 42, "type": "private" },
            "from": { "id": 42, "is_bot": false, "first_name": "Student" },
            "text": "/help"
        }))
        .unwrap();
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://postgres:postgres@localhost/hw-bot")
            .unwrap();
        let dialogue = Dialogue::new(InMemStorage::new(), ChatId(42));

        handle_command(
            bot,
            message,
            Command::Help,
            dialogue,
            ApplicationContainer::new(pool),
        )
        .await
        .unwrap();
    }
}
