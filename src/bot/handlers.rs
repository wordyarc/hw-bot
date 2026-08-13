use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDate, Utc};
use teloxide::dispatching::dialogue::InMemStorageError;
use teloxide::payloads::{
    AnswerCallbackQuerySetters, EditMessageReplyMarkupSetters, SendMessageSetters,
    SetMyCommandsSetters,
};
use teloxide::prelude::{Bot, Requester};
use teloxide::types::{
    BotCommand, BotCommandScope, CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup,
    Message, MessageId, Recipient, User,
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

use super::assignment_files;
use super::errors::send_app_error;
use super::state::{BotDialogue, BotState, DeadlineTarget};
use super::timezone::BotTimezone;
use super::views::{
    BUTTON_ARCHIVE, BUTTON_ASSIGNMENTS, BUTTON_COURSES, BUTTON_QUEUE, BUTTON_REVIEW,
    EXTRA_ATTEMPTS_PREFIX, MAX_ATTEMPTS_PREFIX, MenuAction, ask_input, assignment_actions_keyboard,
    assignment_status, assignment_students_keyboard, attempts_keyboard, button_label,
    calendar_keyboard, course_assignments_keyboard, courses_keyboard, deadline_left,
    deadline_options_keyboard, group_keyboard, main_keyboard, menu_action, parse_month_key,
    review_keyboard, role_name, send_message_chunks, send_stale_button, student_keyboard,
    student_menu, submission_status, teacher_keyboard, teacher_menu,
};
use super::{BotServices, HandlerResult};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub(super) enum Command {
    #[command(description = "🏠 начать работу и открыть меню")]
    Start,
    #[command(description = "🏠 главное меню")]
    Menu,
    #[command(description = "📚 мои задания")]
    Tasks,
    #[command(description = "⏳ мои работы на проверке")]
    Queue,
    #[command(description = "🗄 архив заданий")]
    Archive,
    #[command(description = "✅ проверить следующую работу")]
    Review,
    #[command(description = "📘 мои курсы")]
    Courses,
    #[command(description = "👤 профиль")]
    Profile,
    #[command(description = "❌ отменить текущее действие")]
    Cancel,
    #[command(description = "❓ помощь")]
    Help,
}

const STUDENT_COMMANDS: [&str; 7] = [
    "menu", "tasks", "queue", "archive", "profile", "cancel", "help",
];
const TEACHER_COMMANDS: [&str; 6] = ["menu", "review", "courses", "profile", "cancel", "help"];
const GUEST_COMMANDS: [&str; 3] = ["start", "menu", "help"];

pub(super) fn default_commands() -> Vec<BotCommand> {
    scoped_commands(None)
}

fn scoped_commands(role: Option<Role>) -> Vec<BotCommand> {
    let allowed: &[&str] = match role {
        Some(Role::Student { .. }) => &STUDENT_COMMANDS,
        Some(Role::Teacher | Role::Admin) => &TEACHER_COMMANDS,
        None => &GUEST_COMMANDS,
    };

    Command::bot_commands()
        .into_iter()
        .filter(|command| allowed.contains(&command.command.trim_start_matches('/')))
        .collect()
}

async fn sync_commands(bot: &Bot, chat_id: ChatId, role: Role) {
    if let Err(error) = bot
        .set_my_commands(scoped_commands(Some(role)))
        .scope(BotCommandScope::Chat {
            chat_id: Recipient::Id(chat_id),
        })
        .await
    {
        tracing::warn!(error = ?error, "failed to publish role command hints");
    }
}

#[derive(Clone, Copy)]
struct Screen<'a> {
    bot: &'a Bot,
    chat_id: ChatId,
    actor: Actor,
    services: &'a BotServices,
    dialogue: &'a BotDialogue,
    timezone: &'a BotTimezone,
}

async fn reset_dialogue(dialogue: &BotDialogue) -> HandlerResult {
    match dialogue.exit().await {
        Ok(()) | Err(InMemStorageError::DialogueNotFound) => Ok(()),
    }
}

pub(super) async fn handle_command(
    bot: Bot,
    message: Message,
    command: Command,
    dialogue: BotDialogue,
    services: BotServices,
    timezone: BotTimezone,
) -> HandlerResult {
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    let telegram_user_id = actor_from_message(&message)?;
    let chat_id = message.chat.id;

    let action = match command {
        Command::Start | Command::Menu => {
            reset_dialogue(&dialogue).await?;
            show_home(&bot, chat_id, telegram_user_id, &services, &dialogue).await?;
            return Ok(());
        }
        Command::Cancel => {
            reset_dialogue(&dialogue).await?;
            bot.send_message(chat_id, "Текущее действие отменено.")
                .await?;
            show_home(&bot, chat_id, telegram_user_id, &services, &dialogue).await?;
            return Ok(());
        }
        Command::Help => {
            send_help(&bot, chat_id, telegram_user_id, &services, &timezone).await?;
            return Ok(());
        }
        Command::Tasks => MenuAction::Assignments,
        Command::Queue => MenuAction::Queue,
        Command::Archive => MenuAction::Archive,
        Command::Review => MenuAction::Review,
        Command::Courses => MenuAction::Courses,
        Command::Profile => MenuAction::Profile,
    };

    reset_dialogue(&dialogue).await?;
    run_menu_action(
        &bot,
        chat_id,
        telegram_user_id,
        action,
        &services,
        &dialogue,
        &timezone,
    )
    .await
}

/// Помощь по своей роли: чужие разделы студенту и преподавателю только мешают, а команду из
/// подсказки Telegram делает кликабельной - нажатие на чужую заканчивалось отказом в правах.
async fn send_help(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: TelegramUserId,
    services: &BotServices,
    timezone: &BotTimezone,
) -> HandlerResult {
    let role = match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(registered) => Some(registered.user.role()),
        Err(error) => {
            // Незарегистрированный - обычное дело, а недоступная база не повод не показать
            // справку: в обоих случаях остаётся общий текст.
            if error.code() != ErrorCode::NotFound {
                tracing::warn!(error = ?error, "failed to resolve the role for help");
            }
            None
        }
    };
    let text = format!(
        "{}\n\n🕒 Даты показываются и вводятся в поясе {}.\n\n{}",
        help_for(role),
        timezone.suffix(),
        help_commands(role),
    );

    send_message_chunks(bot, chat_id, text, None).await
}

fn help_for(role: Option<Role>) -> String {
    match role {
        Some(Role::Student { .. }) => format!(
            "❓ Как сдавать работы\n\n\
             1. «{BUTTON_ASSIGNMENTS}» — список заданий с дедлайнами и остатком попыток.\n\
             2. Откройте задание, нажмите «📤 Сдать работу» и пришлите ссылку или текст одним \
             сообщением.\n\
             3. «{BUTTON_QUEUE}» — ваш номер в очереди проверки, чтобы видеть, что работа не \
             потерялась.\n\
             4. Решение придёт отдельным сообщением с комментарием преподавателя. Отклонённую \
             работу можно сдать заново, пока есть попытки и не прошёл дедлайн.\n\
             5. «{BUTTON_ARCHIVE}» — закрытые задания вместе с их историей.",
        ),
        Some(Role::Teacher | Role::Admin) => format!(
            "❓ Как вести курс\n\n\
             1. «{BUTTON_COURSES}» — ваши курсы и задания в них.\n\
             2. «➕ Создать задание» проведёт по шагам: название, текст, попытки, дедлайн, \
             вложения. Черновик виден только вам, студенты получат задание после «🚀 \
             Опубликовать».\n\
             3. «{BUTTON_REVIEW}» выдаёт следующую сдачу из очереди. Решение требует \
             комментария — студент увидит его в уведомлении.\n\
             4. В карточке задания меняется дедлайн, выдаются дополнительные попытки студенту и \
             добавляются вложения.",
        ),
        None => "❓ Бот собирает домашние работы: преподаватель выдаёт задания с дедлайном, \
                 студент сдаёт их одним сообщением.\n\n\
                 Отправьте /start, чтобы зарегистрироваться."
            .to_owned(),
    }
}

fn help_commands(role: Option<Role>) -> String {
    scoped_commands(role)
        .into_iter()
        .map(|command| {
            format!(
                "/{} — {}\n",
                command.command.trim_start_matches('/'),
                command.description,
            )
        })
        .collect()
}

pub(super) async fn handle_message(
    bot: Bot,
    message: Message,
    dialogue: BotDialogue,
    services: BotServices,
    transfers: super::FileTransfers,
    timezone: BotTimezone,
) -> HandlerResult {
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    let actor = actor_from_message(&message)?;
    let state = dialogue.get().await?.unwrap_or_default();

    if let Some(action) = message.text().and_then(menu_action) {
        reset_dialogue(&dialogue).await?;
        return run_menu_action(
            &bot,
            message.chat.id,
            actor,
            action,
            &services,
            &dialogue,
            &timezone,
        )
        .await;
    }

    if assignment_files::handle_message(
        &bot, &message, &state, &dialogue, &services, &transfers, actor,
    )
    .await?
    {
        return Ok(());
    }

    match state {
        BotState::Idle => {
            bot.send_message(
                message.chat.id,
                "Используйте меню под полем ввода или команду /menu.",
            )
            .await?;
        }
        BotState::AwaitingRegistrationName => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
                    reset_dialogue(&dialogue).await?;
                    bot.send_message(
                        message.chat.id,
                        format!(
                            "✅ Работа отправлена на проверку. Место в очереди видно в «{BUTTON_QUEUE}»."
                        ),
                    )
                    .reply_markup(student_keyboard())
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
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
            let Some(reviewer) = resolve_actor(&bot, message.chat.id, actor, &services).await?
            else {
                return Ok(());
            };
            let result = services
                .reviews
                .review_submission(ReviewSubmission {
                    submission_id,
                    actor: reviewer,
                    decision,
                    comment: text.to_owned(),
                    reviewed_at: Utc::now(),
                })
                .await;

            match result {
                Ok(_) => {
                    reset_dialogue(&dialogue).await?;
                    bot.send_message(
                        message.chat.id,
                        "✅ Решение сохранено, студент получит уведомление.",
                    )
                    .reply_markup(teacher_keyboard())
                    .await?;
                }
                Err(error) if error.code() == ErrorCode::SubmissionAlreadyReviewed => {
                    reset_dialogue(&dialogue).await?;
                    send_app_error(&bot, message.chat.id, error).await?;
                    show_home(&bot, message.chat.id, actor, &services, &dialogue).await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::ManagingAssignment { .. } => {
            bot.send_message(message.chat.id, "Выберите действие кнопкой под заданием.")
                .await?;
        }
        BotState::ManagingAssignmentFiles { .. }
        | BotState::AwaitingAssignmentFile { .. }
        | BotState::ConfirmingAssignmentFileDeletion { .. } => unreachable!(),
        BotState::AwaitingAssignmentTitle { course_id } => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
            ask_input(
                &bot,
                message.chat.id,
                "2/4. Пришлите текст задания одним сообщением:",
            )
            .await?;
        }
        BotState::AwaitingAssignmentBody { course_id, title } => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
            ask_max_attempts(
                &bot,
                message.chat.id,
                course_id,
                title,
                text.to_owned(),
                &dialogue,
            )
            .await?;
        }
        BotState::AwaitingAssignmentMaxAttempts {
            course_id,
            title,
            body,
            ..
        } => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
            let Some(max_attempts) = parse_positive_attempts(text) else {
                bot.send_message(
                    message.chat.id,
                    format!("Введите целое число от 1 до {MAX_ATTEMPTS}."),
                )
                .await?;
                return Ok(());
            };
            ask_deadline(
                &bot,
                message.chat.id,
                DeadlineTarget::Create {
                    course_id,
                    title,
                    body,
                    max_attempts,
                },
                &dialogue,
                &timezone,
            )
            .await?;
        }
        // Пока висит клавиатура с пресетами, набранная дата - это тоже ответ, а не мимо-сообщение.
        BotState::ChoosingDeadline { target, .. }
        | BotState::PickingDeadlineDate { target, .. }
        | BotState::AwaitingDeadlineText { target } => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
            let Some(deadline_at) = timezone.parse_deadline(text) else {
                send_deadline_format_error(&bot, message.chat.id, &timezone).await?;
                return Ok(());
            };
            let Some(actor) = resolve_actor(&bot, message.chat.id, actor, &services).await? else {
                return Ok(());
            };
            apply_deadline(
                Screen {
                    bot: &bot,
                    chat_id: message.chat.id,
                    actor,
                    services: &services,
                    dialogue: &dialogue,
                    timezone: &timezone,
                },
                target,
                deadline_at,
            )
            .await?;
        }
        BotState::ChoosingAttemptStudent { .. } => {
            bot.send_message(message.chat.id, "Выберите студента кнопкой выше.")
                .await?;
        }
        BotState::AwaitingAttemptAmount {
            assignment_id,
            student_id,
            ..
        } => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
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
            grant_attempts(
                Screen {
                    bot: &bot,
                    chat_id: message.chat.id,
                    actor,
                    services: &services,
                    dialogue: &dialogue,
                    timezone: &timezone,
                },
                assignment_id,
                student_id,
                amount,
            )
            .await?;
        }
    }

    Ok(())
}

async fn run_menu_action(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: TelegramUserId,
    action: MenuAction,
    services: &BotServices,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    if action == MenuAction::Help {
        return send_help(bot, chat_id, telegram_user_id, services, timezone).await;
    }

    let Some(actor) = resolve_actor(bot, chat_id, telegram_user_id, services).await? else {
        return Ok(());
    };
    let screen = Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        timezone,
    };

    match action {
        MenuAction::Assignments => show_student_assignments(screen).await,
        MenuAction::Archive => show_student_archive(screen).await,
        MenuAction::Queue => show_student_queue(screen).await,
        MenuAction::Review => show_next_review(screen).await,
        MenuAction::Courses => show_courses(screen).await,
        MenuAction::Profile => show_profile(screen).await,
        MenuAction::Help => Ok(()),
    }
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
    transfers: super::FileTransfers,
    timezone: BotTimezone,
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

    if data == "cal:noop" {
        return Ok(());
    }

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
                reset_dialogue(&dialogue).await?;
                bot.send_message(chat_id, "🎉 Регистрация завершена.")
                    .await?;
                show_home(&bot, chat_id, telegram_user_id, &services, &dialogue).await?;
            }
            Err(error) => send_app_error(&bot, chat_id, error).await?,
        }
        return Ok(());
    }

    let Some(actor) = resolve_actor(&bot, chat_id, telegram_user_id, &services).await? else {
        return Ok(());
    };
    if assignment_files::handle_callback(assignment_files::CallbackContext {
        bot: &bot,
        chat_id,
        message_id: message.id,
        data,
        state: &state,
        actor,
        dialogue: &dialogue,
        services: &services,
        transfers: &transfers,
    })
    .await?
    {
        return Ok(());
    }

    let screen = Screen {
        bot: &bot,
        chat_id,
        actor,
        services: &services,
        dialogue: &dialogue,
        timezone: &timezone,
    };

    match data {
        "menu:assignments" => show_student_assignments(screen).await?,
        "menu:archive" => show_student_archive(screen).await?,
        "menu:queue" => show_student_queue(screen).await?,
        "menu:profile" => show_profile(screen).await?,
        "menu:courses" => show_courses(screen).await?,
        "review:next" => show_next_review(screen).await?,
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
            ask_input(
                &bot,
                chat_id,
                "Напишите обязательный комментарий к решению:",
            )
            .await?;
        }
        _ if data.starts_with("course:") => {
            let Some(id) = parse_uuid_callback(data, "course:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_course_assignments(screen, TeachingCourseId::from_uuid(id)).await?;
        }
        _ if data.starts_with("manage:") => {
            let Some(id) = parse_uuid_callback(data, "manage:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_teacher_assignment(screen, AssignmentId::from_uuid(id)).await?;
        }
        _ if data.starts_with("create:") => {
            let Some(id) = parse_uuid_callback(data, "create:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            dialogue
                .update(BotState::AwaitingAssignmentTitle {
                    course_id: TeachingCourseId::from_uuid(id),
                })
                .await?;
            ask_input(&bot, chat_id, "1/4. Введите название задания:").await?;
        }
        _ if data.starts_with(MAX_ATTEMPTS_PREFIX) => {
            let BotState::AwaitingAssignmentMaxAttempts {
                message_id,
                course_id,
                title,
                body,
            } = state
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if message_id != message.id {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            let Some(max_attempts) = data
                .strip_prefix(MAX_ATTEMPTS_PREFIX)
                .and_then(parse_positive_attempts)
            else {
                ask_input(
                    &bot,
                    chat_id,
                    &format!("Введите количество попыток от 1 до {MAX_ATTEMPTS}:"),
                )
                .await?;
                return Ok(());
            };
            ask_deadline(
                &bot,
                chat_id,
                DeadlineTarget::Create {
                    course_id,
                    title,
                    body,
                    max_attempts,
                },
                &dialogue,
                &timezone,
            )
            .await?;
        }
        _ if data.starts_with("dl:") || data.starts_with("cal:") => {
            handle_deadline_callback(screen, message.id, data, state).await?;
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
                    bot.send_message(chat_id, "🚀 Задание опубликовано.")
                        .await?;
                    show_teacher_assignment(screen, assignment_id).await?;
                }
                Err(error) => send_app_error(&bot, chat_id, error).await?,
            }
        }
        "file:finish" => {
            let BotState::ManagingAssignmentFiles {
                message_id,
                assignment_id,
                creation_flow: true,
                ..
            } = state
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if message_id != message.id {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            show_teacher_assignment(screen, assignment_id).await?;
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
                    bot.send_message(chat_id, "📦 Задание архивировано.")
                        .await?;
                    show_teacher_assignment(screen, assignment_id).await?;
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
            ask_deadline(
                &bot,
                chat_id,
                DeadlineTarget::Change { assignment_id },
                &dialogue,
                &timezone,
            )
            .await?;
        }
        _ if data.starts_with("attempts:") => {
            let Some(id) = parse_uuid_callback(data, "attempts:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_assignment_students(screen, AssignmentId::from_uuid(id)).await?;
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
            let sent = bot
                .send_message(chat_id, "Сколько дополнительных попыток выдать?")
                .reply_markup(attempts_keyboard(EXTRA_ATTEMPTS_PREFIX))
                .await?;
            dialogue
                .update(BotState::AwaitingAttemptAmount {
                    message_id: sent.id,
                    assignment_id,
                    student_id,
                })
                .await?;
        }
        _ if data.starts_with(EXTRA_ATTEMPTS_PREFIX) => {
            let BotState::AwaitingAttemptAmount {
                message_id,
                assignment_id,
                student_id,
            } = state
            else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            if message_id != message.id {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            }
            let Some(amount) = data
                .strip_prefix(EXTRA_ATTEMPTS_PREFIX)
                .and_then(parse_positive_attempts)
            else {
                ask_input(
                    &bot,
                    chat_id,
                    &format!("Введите целое число от 1 до {MAX_ATTEMPTS}:"),
                )
                .await?;
                return Ok(());
            };
            grant_attempts(screen, assignment_id, student_id, amount).await?;
        }
        _ if data.starts_with("assignment:") => {
            let Some(id) = parse_uuid_callback(data, "assignment:") else {
                send_stale_button(&bot, chat_id).await?;
                return Ok(());
            };
            show_assignment(screen, AssignmentId::from_uuid(id), &transfers).await?;
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
            ask_input(
                &bot,
                chat_id,
                "Пришлите ссылку или текст сдачи одним сообщением:",
            )
            .await?;
        }
        _ => send_stale_button(&bot, chat_id).await?,
    }

    Ok(())
}

async fn handle_deadline_callback(
    screen: Screen<'_>,
    message_id: MessageId,
    data: &str,
    state: BotState,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        dialogue,
        timezone,
        ..
    } = screen;
    let (current_message_id, target) = match state {
        BotState::ChoosingDeadline { message_id, target }
        | BotState::PickingDeadlineDate { message_id, target } => (message_id, target),
        _ => {
            send_stale_button(bot, chat_id).await?;
            return Ok(());
        }
    };
    if current_message_id != message_id {
        send_stale_button(bot, chat_id).await?;
        return Ok(());
    }

    let date = if let Some(days) = data.strip_prefix("dl:days:") {
        timezone.in_days(days.parse().unwrap_or(0))
    } else if let Some(months) = data.strip_prefix("dl:months:") {
        timezone.in_months(months.parse().unwrap_or(0))
    } else if let Some(date) = data.strip_prefix("cal:pick:") {
        NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
    } else {
        match data {
            "dl:manual" => {
                dialogue
                    .update(BotState::AwaitingDeadlineText { target })
                    .await?;
                ask_input(
                    bot,
                    chat_id,
                    &format!(
                        "Введите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ ({}):",
                        timezone.suffix()
                    ),
                )
                .await?;
                return Ok(());
            }
            "dl:options" => {
                let sent = bot
                    .send_message(chat_id, deadline_prompt(&target, timezone))
                    .reply_markup(deadline_options_keyboard(timezone))
                    .await?;
                dialogue
                    .update(BotState::ChoosingDeadline {
                        message_id: sent.id,
                        target,
                    })
                    .await?;
                return Ok(());
            }
            "dl:calendar" => {
                let month = timezone.today();
                bot.edit_message_reply_markup(chat_id, message_id)
                    .reply_markup(calendar_keyboard(month, month))
                    .await?;
                dialogue
                    .update(BotState::PickingDeadlineDate { message_id, target })
                    .await?;
                return Ok(());
            }
            _ => {
                let Some(month) = data.strip_prefix("cal:").and_then(parse_month_key) else {
                    send_stale_button(bot, chat_id).await?;
                    return Ok(());
                };
                bot.edit_message_reply_markup(chat_id, message_id)
                    .reply_markup(calendar_keyboard(month, timezone.today()))
                    .await?;
                dialogue
                    .update(BotState::PickingDeadlineDate { message_id, target })
                    .await?;
                return Ok(());
            }
        }
    };

    let Some(deadline_at) = date.and_then(|date| timezone.end_of(date)) else {
        send_stale_button(bot, chat_id).await?;
        return Ok(());
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

async fn ask_max_attempts(
    bot: &Bot,
    chat_id: ChatId,
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
        .update(BotState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            course_id,
            title,
            body,
        })
        .await?;
    Ok(())
}

async fn ask_deadline(
    bot: &Bot,
    chat_id: ChatId,
    target: DeadlineTarget,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, deadline_prompt(&target, timezone))
        .reply_markup(deadline_options_keyboard(timezone))
        .await?;
    dialogue
        .update(BotState::ChoosingDeadline {
            message_id: sent.id,
            target,
        })
        .await?;
    Ok(())
}

async fn apply_deadline(
    screen: Screen<'_>,
    target: DeadlineTarget,
    deadline_at: DateTime<Utc>,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        timezone,
    } = screen;
    match target {
        DeadlineTarget::Create {
            course_id,
            title,
            body,
            max_attempts,
        } => {
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
                    bot.send_message(
                        chat_id,
                        format!(
                            "📝 Черновик создан. Дедлайн: {}",
                            timezone.format(deadline_at)
                        ),
                    )
                    .reply_markup(teacher_keyboard())
                    .await?;
                    assignment_files::show(
                        bot,
                        chat_id,
                        actor,
                        assignment.id(),
                        true,
                        services,
                        dialogue,
                    )
                    .await?;
                }
                Err(error) => send_app_error(bot, chat_id, error).await?,
            }
        }
        DeadlineTarget::Change { assignment_id } => {
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
                    bot.send_message(
                        chat_id,
                        format!("⏰ Дедлайн изменён: {}", timezone.format(deadline_at)),
                    )
                    .reply_markup(teacher_keyboard())
                    .await?;
                    show_teacher_assignment(screen, assignment_id).await?;
                }
                Err(error) => send_app_error(bot, chat_id, error).await?,
            }
        }
    }

    Ok(())
}

async fn grant_attempts(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    student_id: UserId,
    amount: u16,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        ..
    } = screen;
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
                chat_id,
                format!(
                    "🎁 Добавлено попыток: {amount}. Всего дополнительных: {}.",
                    progress.extra_attempts()
                ),
            )
            .reply_markup(teacher_keyboard())
            .await?;
            show_assignment_students(screen, assignment_id).await?;
        }
        Err(error) => send_app_error(bot, chat_id, error).await?,
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
        Ok(registered) => {
            let role = registered.user.role();
            sync_commands(bot, chat_id, role).await;
            let name = registered.user.display_name().as_str();
            let text = match role {
                Role::Student { .. } => {
                    let pending = services
                        .submissions
                        .pending_queue(Actor::new(registered.user.id()))
                        .await
                        .map(|places| places.len())
                        .unwrap_or_default();
                    if pending == 0 {
                        format!("👋 Здравствуйте, {name}!\nВыберите пункт меню ниже 👇")
                    } else {
                        format!(
                            "👋 Здравствуйте, {name}!\n⏳ На проверке ваших работ: {pending}\nВыберите пункт меню ниже 👇"
                        )
                    }
                }
                Role::Teacher | Role::Admin => {
                    let pending = match services
                        .reviews
                        .queue_size(Actor::new(registered.user.id()))
                        .await
                    {
                        Ok(pending) => pending,
                        Err(error) => {
                            send_app_error(bot, chat_id, error).await?;
                            return Ok(());
                        }
                    };
                    format!(
                        "👋 Здравствуйте, {name}!\n⏳ Ждут проверки: {pending}\nВыберите пункт меню ниже 👇"
                    )
                }
            };
            bot.send_message(chat_id, text)
                .reply_markup(main_keyboard(role))
                .await?;
        }
        Err(error) if error.code() == ErrorCode::NotFound => {
            dialogue.update(BotState::AwaitingRegistrationName).await?;
            // Без `ask_input`: у незарегистрированного нижнего меню ещё нет, а отменять регистрацию
            // некуда - до неё бот всё равно ничего не умеет.
            bot.send_message(chat_id, "Вы ещё не зарегистрированы. Как вас называть?")
                .await?;
        }
        Err(error) => send_app_error(bot, chat_id, error).await?,
    }

    Ok(())
}

async fn show_profile(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        timezone,
        ..
    } = screen;
    let registered = match services.users.find_by_actor(actor).await {
        Ok(registered) => registered,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let role = registered.user.role();
    let mut text = format!(
        "👤 Профиль\n\nИмя: {}\nРоль: {}\n",
        registered.user.display_name().as_str(),
        role_name(role),
    );
    if let Some(group) = &registered.student_group {
        text.push_str(&format!("Группа: {}\n", group.code().as_str()));
    }
    text.push_str(&format!("🕒 Часовой пояс: {}\n", timezone.suffix()));

    match role {
        Role::Student { .. } => {
            if let Ok(assignments) = services.assignments.list_for_student(actor).await {
                text.push_str(&format!("\n📚 Активных заданий: {}\n", assignments.len()));
            }
            if let Ok(places) = services.submissions.pending_queue(actor).await {
                text.push_str(&format!("⏳ Работ на проверке: {}\n", places.len()));
            }
        }
        Role::Teacher | Role::Admin => {
            if let Ok(courses) = services.courses.list_for_teacher(actor).await {
                text.push_str(&format!("\n📘 Курсов: {}\n", courses.len()));
            }
            if let Ok(pending) = services.reviews.queue_size(actor).await {
                text.push_str(&format!("⏳ Ждут проверки: {pending}\n"));
            }
        }
    }

    let keyboard = match role {
        Role::Student { .. } => student_menu(),
        Role::Teacher | Role::Admin => teacher_menu(None),
    };
    send_message_chunks(bot, chat_id, text, Some(keyboard)).await
}

async fn show_student_queue(screen: Screen<'_>) -> HandlerResult {
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
        bot.send_message(chat_id, "⏳ Сейчас ни одна ваша работа не ждёт проверки.")
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

/// Позиция `0` означает, что очередь ещё некому разбирать: у курса нет преподавателя.
fn queue_place(position: u32, total: u32) -> String {
    if position == 0 {
        "преподаватель ещё не назначен на курс".to_owned()
    } else {
        format!("ваш номер в очереди: {position} из {total}")
    }
}

/// Заголовок предмета печатается один раз на группу: список студента отсортирован по предметам,
/// иначе задания нескольких курсов читаются одной кучей. Кнопки заголовок не видят, поэтому предмет
/// дублируется в подписи каждой.
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

async fn show_student_assignments(screen: Screen<'_>) -> HandlerResult {
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
            deadline_left(assignment.deadline_at()),
            remaining,
        ));
        rows.push(vec![InlineKeyboardButton::callback(
            button_label(&format!("{subject} · {}", assignment.title().as_str())),
            format!("assignment:{}", assignment.id().as_uuid()),
        )]);
    }

    send_message_chunks(bot, chat_id, text, Some(InlineKeyboardMarkup::new(rows))).await?;
    Ok(())
}

async fn show_student_archive(screen: Screen<'_>) -> HandlerResult {
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
            format!("assignment:{}", assignment.id().as_uuid()),
        )]);
    }

    send_message_chunks(bot, chat_id, text, Some(InlineKeyboardMarkup::new(rows))).await?;
    Ok(())
}

async fn show_assignment(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    transfers: &super::FileTransfers,
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
        "📘 {}\n{}\n\n⏰ Дедлайн: {} ({})\n🔁 Попытки: использовано {}, доступно {}, осталось {}",
        details.assignment.subject.name().as_str(),
        assignment.title().as_str(),
        timezone.format(assignment.deadline_at()),
        deadline_left(assignment.deadline_at()),
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
            format!("submit:{}", assignment.id().as_uuid()),
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
                "Попытка {}: {}",
                submission.attempt_number(),
                submission_status(submission.status()),
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

async fn show_courses(screen: Screen<'_>) -> HandlerResult {
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
    reset_dialogue(dialogue).await?;
    Ok(())
}

async fn show_course_assignments(screen: Screen<'_>, course_id: TeachingCourseId) -> HandlerResult {
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
    reset_dialogue(dialogue).await?;
    Ok(())
}

async fn show_teacher_assignment(screen: Screen<'_>, assignment_id: AssignmentId) -> HandlerResult {
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
        deadline_left(assignment.deadline_at()),
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
            assignment_id,
        })
        .await?;
    Ok(())
}

async fn show_assignment_students(
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
        .update(BotState::ChoosingAttemptStudent {
            message_id: message.id,
            assignment_id,
            student_ids: students.iter().map(|item| item.student.id()).collect(),
        })
        .await?;
    Ok(())
}

async fn show_next_review(screen: Screen<'_>) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let item = match services.reviews.take_next_from_queue(actor).await {
        Ok(item) => item,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    let Some(item) = item else {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, "🎉 Очередь проверки пуста.")
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
        "{}\n👤 Студент: {}",
        item.assignment.title().as_str(),
        item.student.display_name().as_str(),
    );
    let submission_id = item.submission.id();
    let keyboard = review_keyboard(submission_id);
    bot.send_message(chat_id, summary).await?;
    bot.send_message(chat_id, "📄 Текст задания:").await?;
    bot.send_message(chat_id, item.assignment.body().as_str())
        .await?;
    bot.send_message(chat_id, "📤 Сдача студента:").await?;
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

async fn require_text<'a>(bot: &Bot, message: &'a Message) -> HandlerResult<Option<&'a str>> {
    let Some(text) = message.text() else {
        bot.send_message(message.chat.id, "Пришлите текстовое сообщение.")
            .await?;
        return Ok(None);
    };
    Ok(Some(text))
}

fn parse_positive_attempts(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value: u16 = text.parse().ok()?;

    (value > 0 && value <= MAX_ATTEMPTS).then_some(value)
}

async fn send_deadline_format_error(
    bot: &Bot,
    chat_id: ChatId,
    timezone: &BotTimezone,
) -> HandlerResult {
    bot.send_message(
        chat_id,
        format!(
            "Не удалось разобрать дату. Используйте ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ в поясе {}.",
            timezone.suffix()
        ),
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
    use chrono::{FixedOffset, NaiveTime};
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::app::{ApplicationContainer, FilePolicy};
    use crate::domain::StudentGroupId;

    use super::*;

    fn timezone() -> BotTimezone {
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
    }

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
    fn every_menu_screen_is_reachable_by_a_command() {
        for (input, expected) in [
            ("/tasks", MenuAction::Assignments),
            ("/queue", MenuAction::Queue),
            ("/archive", MenuAction::Archive),
            ("/review", MenuAction::Review),
            ("/courses", MenuAction::Courses),
            ("/profile", MenuAction::Profile),
        ] {
            let action = match Command::parse(input, "test_bot").unwrap() {
                Command::Tasks => MenuAction::Assignments,
                Command::Queue => MenuAction::Queue,
                Command::Archive => MenuAction::Archive,
                Command::Review => MenuAction::Review,
                Command::Courses => MenuAction::Courses,
                Command::Profile => MenuAction::Profile,
                _ => panic!("{input} must open a menu screen"),
            };

            assert_eq!(action, expected, "{input}");
        }
    }

    #[test]
    fn command_hints_are_scoped_to_the_role() {
        let student = names(scoped_commands(Some(Role::Student {
            student_group_id: StudentGroupId::new(),
        })));
        let teacher = names(scoped_commands(Some(Role::Teacher)));

        assert!(student.contains(&"queue".to_owned()));
        assert!(!student.contains(&"review".to_owned()));
        assert!(teacher.contains(&"review".to_owned()));
        assert!(!teacher.contains(&"queue".to_owned()));
        assert!(names(default_commands()).contains(&"start".to_owned()));
    }

    #[test]
    fn help_tells_each_role_only_about_its_own_half_of_the_bot() {
        let student_role = Some(Role::Student {
            student_group_id: StudentGroupId::new(),
        });
        let student = format!("{}{}", help_for(student_role), help_commands(student_role));
        let teacher = format!(
            "{}{}",
            help_for(Some(Role::Teacher)),
            help_commands(Some(Role::Teacher))
        );
        let guest = format!("{}{}", help_for(None), help_commands(None));

        assert!(student.contains("/queue"), "{student}");
        assert!(!student.contains("/review"), "{student}");
        assert!(!student.contains("/courses"), "{student}");
        assert!(teacher.contains("/review"), "{teacher}");
        assert!(!teacher.contains("/tasks"), "{teacher}");
        assert!(!teacher.contains("/queue"), "{teacher}");
        assert!(guest.contains("/start"), "{guest}");
    }

    #[test]
    fn every_declared_command_is_offered_to_someone() {
        let offered: Vec<_> = [
            scoped_commands(None),
            scoped_commands(Some(Role::Teacher)),
            scoped_commands(Some(Role::Student {
                student_group_id: StudentGroupId::new(),
            })),
        ]
        .into_iter()
        .flat_map(names)
        .collect();

        for command in names(Command::bot_commands()) {
            assert!(offered.contains(&command), "{command} is unreachable");
        }
    }

    fn names(commands: Vec<BotCommand>) -> Vec<String> {
        commands
            .into_iter()
            .map(|command| command.command.trim_start_matches('/').to_owned())
            .collect()
    }

    const ROUTES: [&str; 23] = [
        "menu:assignments",
        "menu:archive",
        "menu:queue",
        "menu:profile",
        "menu:courses",
        "review:next",
        "review:accept:",
        "review:reject:",
        "register_group:",
        "course:",
        "manage:",
        "create:",
        MAX_ATTEMPTS_PREFIX,
        EXTRA_ATTEMPTS_PREFIX,
        "dl:",
        "cal:",
        "publish:",
        "archive:",
        "deadline:",
        "attempts:",
        "student:",
        "files:",
        "file:",
    ];

    #[test]
    fn every_button_the_bot_sends_is_routable() {
        use crate::bot::views::{
            assignment_files_keyboard, assignment_students_keyboard, attempts_keyboard,
            calendar_keyboard, confirm_file_deletion_keyboard, course_assignments_keyboard,
            deadline_options_keyboard, group_keyboard,
        };
        use crate::domain::{
            FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner, StoredFileParts,
        };
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let assignment = assignment(&course, &teacher).published().build();
        let file = StoredFileMetadata::restore(StoredFileParts {
            id: StoredFileId::new(),
            owner: StoredFileOwner::Assignment(assignment.id()),
            position: 1,
            presentation: FilePresentation::Document,
            filename: "file.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            size_bytes: 4,
            sha256: [0; 32],
            created_at: Utc::now(),
        })
        .unwrap();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let timezone = timezone();

        let keyboards = [
            student_menu(),
            teacher_menu(Some(1)),
            group_keyboard(&["б9122".to_owned()]),
            review_keyboard(SubmissionId::new()),
            course_assignments_keyboard(course.id(), std::slice::from_ref(&assignment)),
            assignment_actions_keyboard(&assignment),
            attempts_keyboard(MAX_ATTEMPTS_PREFIX),
            attempts_keyboard(EXTRA_ATTEMPTS_PREFIX),
            deadline_options_keyboard(&timezone),
            calendar_keyboard(today, today),
            assignment_files_keyboard(&assignment, &[file], true, true),
            assignment_files_keyboard(&assignment, &[], true, false),
            confirm_file_deletion_keyboard(),
            assignment_students_keyboard(assignment.id(), &[]),
        ];

        for keyboard in keyboards {
            let json = serde_json::to_value(keyboard).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    let data = button["callback_data"].as_str().unwrap();
                    assert!(
                        ROUTES.iter().any(|route| data.starts_with(route)),
                        "button {:?} sends {data}, which no handler matches",
                        button["text"],
                    );
                    assert!(data.len() <= 64, "{data} exceeds the Telegram limit");
                }
            }
        }
    }

    #[test]
    fn the_attempts_keyboard_speaks_the_language_of_the_step_that_sent_it() {
        for (prefix, expected) in [
            (MAX_ATTEMPTS_PREFIX, "maxattempts:3"),
            (EXTRA_ATTEMPTS_PREFIX, "attempt:3"),
        ] {
            let json = serde_json::to_value(super::attempts_keyboard(prefix)).unwrap();
            let third = json["inline_keyboard"][0][2]["callback_data"]
                .as_str()
                .unwrap()
                .to_owned();

            assert_eq!(third, expected);
            assert_eq!(
                third.strip_prefix(prefix).and_then(parse_positive_attempts),
                Some(3)
            );
        }
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
        let timezone = timezone();

        assert_eq!(
            timezone.parse_deadline("31.12.2030 23:59").unwrap(),
            "2030-12-31T13:59:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert!(timezone.parse_deadline("31.12.2030 23:59 UTC").is_none());
        assert!(timezone.parse_deadline("31.02.2030 23:59").is_none());

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
    fn a_queue_without_reviewers_does_not_show_a_made_up_position() {
        assert_eq!(queue_place(2, 5), "ваш номер в очереди: 2 из 5");
        assert_eq!(queue_place(0, 0), "преподаватель ещё не назначен на курс");
    }

    #[test]
    fn assignment_action_requires_current_keyboard_and_assignment() {
        let current = AssignmentId::new();
        let other = AssignmentId::new();
        let state = BotState::ManagingAssignment {
            message_id: MessageId(10),
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
        Mock::given(method("POST"))
            .and(path("/bottest-token/SendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {
                    "message_id": 2,
                    "date": 0,
                    "chat": { "id": 42, "type": "private" },
                    "text": "help",
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
            ApplicationContainer::new(
                pool,
                FilePolicy {
                    max_file_size_bytes: 1024,
                    max_files_per_assignment: 5,
                    max_files_per_submission: 1,
                },
            ),
            timezone(),
        )
        .await
        .unwrap();
    }
}
