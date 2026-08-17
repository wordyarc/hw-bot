use crate::app::{Actor, ErrorCode, RegisterStudent, ReviewSubmission, SubmitHomework};
use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, AssignmentId, DISPLAY_NAME_MAX_LEN, Grading, LatePolicy,
    MAX_ATTEMPTS, MAX_SCORE, ReviewDecision, Role, SubmissionId, TelegramUserId, USER_TEXT_MAX_LEN,
};
use anyhow::{Context, anyhow};
use chrono::Utc;
use teloxide::dispatching::dialogue::InMemStorageError;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters, SetMyCommandsSetters};
use teloxide::prelude::{Bot, Requester};
use teloxide::types::{
    BotCommand, BotCommandScope, CallbackQuery, ChatId, Message, MessageId, Recipient, User,
};
use teloxide::utils::command::BotCommands;

use super::assignment_files;
use super::callbacks::{
    self, AmountChoice, CallbackAction, FlowAction, ReplayableAction, ScreenAction,
};
use super::errors::{send_app_error, send_error_code};
use super::invites::BotInvites;
use super::state::{BotDialogue, BotState, DraftAssignment, FlowState, ScreenState};
use super::timezone::BotTimezone;
use super::views::{
    BUTTON_ARCHIVE, BUTTON_ASSIGNMENTS, BUTTON_COURSES, BUTTON_QUEUE, BUTTON_REVIEW, MenuAction,
    ask_input, main_keyboard, menu_action, review_score_keyboard, role_name, send_message_chunks,
    send_stale_button, student_keyboard, student_menu, teacher_keyboard, teacher_menu,
};
use super::{BotServices, HandlerResult};

mod review;
mod student;
mod teacher;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub(super) enum Command {
    #[command(description = "🏠 начать работу и открыть меню")]
    Start(String),
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

const GUEST_NEEDS_INVITE: &str = "Регистрация только по ссылке-приглашению группы. Попросите её у преподавателя и откройте \
     присланную ссылку.";

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

/// Зависимости dispatcher-а, которые нужны ещё до разрешения актора.
#[derive(Clone, Copy)]
struct Deps<'a> {
    services: &'a BotServices,
    dialogue: &'a BotDialogue,
    timezone: &'a BotTimezone,
    invites: &'a BotInvites,
}

#[derive(Clone, Copy)]
pub(super) struct Screen<'a> {
    bot: &'a Bot,
    chat_id: ChatId,
    actor: Actor,
    services: &'a BotServices,
    dialogue: &'a BotDialogue,
    timezone: &'a BotTimezone,
    invites: &'a BotInvites,
}

pub(super) async fn reset_dialogue(dialogue: &BotDialogue) -> HandlerResult {
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
    invites: BotInvites,
) -> HandlerResult {
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    let telegram_user_id = actor_from_message(&message)?;
    let chat_id = message.chat.id;

    let action = match command {
        Command::Start(payload) => {
            reset_dialogue(&dialogue).await?;
            return start_registration(
                &bot,
                chat_id,
                telegram_user_id,
                payload.trim(),
                &services,
                &dialogue,
                &invites,
            )
            .await;
        }
        Command::Menu => {
            reset_dialogue(&dialogue).await?;
            show_home(&bot, chat_id, telegram_user_id, &services).await?;
            return Ok(());
        }
        Command::Cancel => {
            reset_dialogue(&dialogue).await?;
            bot.send_message(chat_id, "Текущее действие отменено.")
                .await?;
            show_home(&bot, chat_id, telegram_user_id, &services).await?;
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
        Deps {
            services: &services,
            dialogue: &dialogue,
            timezone: &timezone,
            invites: &invites,
        },
    )
    .await
}

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
            // Незарегистрированный
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
    invites: BotInvites,
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
            Deps {
                services: &services,
                dialogue: &dialogue,
                timezone: &timezone,
                invites: &invites,
            },
        )
        .await;
    }

    if matches!(
        state,
        BotState::Screen(ScreenState::ManagingAssignmentFiles { .. })
            | BotState::Flow(FlowState::AwaitingAssignmentFile { .. })
            | BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. })
    ) {
        return assignment_files::handle_message(
            &bot, &message, &state, &dialogue, &services, &transfers, actor,
        )
        .await;
    }

    match state {
        BotState::Idle => {
            bot.send_message(
                message.chat.id,
                "Используйте меню под полем ввода или команду /menu.",
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingRegistrationName { invite }) => {
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

            match services
                .users
                .register_student(RegisterStudent {
                    telegram_user_id: actor,
                    display_name: text.trim().to_owned(),
                    invite,
                    registered_at: Utc::now(),
                })
                .await
            {
                Ok(_) => {
                    reset_dialogue(&dialogue).await?;
                    bot.send_message(message.chat.id, "🎉 Регистрация завершена.")
                        .await?;
                    show_home(&bot, message.chat.id, actor, &services).await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::Flow(FlowState::AwaitingSubmission { assignment_id }) => {
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
        BotState::Flow(FlowState::AwaitingReviewDecision { .. }) => {
            bot.send_message(message.chat.id, "Выберите решение кнопкой под сдачей.")
                .await?;
        }
        BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            penalty,
        }) => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
            let Some(score) = parse_score(text, max_score) else {
                bot.send_message(
                    message.chat.id,
                    format!("Введите целое число от 0 до {max_score}."),
                )
                .await?;
                return Ok(());
            };
            confirm_score(&bot, message.chat.id, score, penalty, max_score).await?;
            ask_review_comment(
                &bot,
                message.chat.id,
                submission_id,
                ReviewDecision::Accept,
                Some(score),
                &dialogue,
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score,
        }) => {
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
                    score,
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
                    show_home(&bot, message.chat.id, actor, &services).await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
        BotState::Screen(ScreenState::ManagingAssignment { .. }) => {
            bot.send_message(message.chat.id, "Выберите действие кнопкой под заданием.")
                .await?;
        }
        BotState::Screen(ScreenState::GroupInvite { .. })
        | BotState::Flow(FlowState::ConfirmingInviteRotation { .. }) => {
            bot.send_message(message.chat.id, "Выберите действие кнопкой выше.")
                .await?;
        }
        BotState::Screen(ScreenState::ManagingAssignmentFiles { .. })
        | BotState::Flow(FlowState::AwaitingAssignmentFile { .. })
        | BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. }) => unreachable!(),
        BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id }) => {
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
                .update(BotState::Flow(FlowState::AwaitingAssignmentBody {
                    course_id,
                    title: text.to_owned(),
                }))
                .await?;
            ask_input(
                &bot,
                message.chat.id,
                "2/5. Пришлите текст задания одним сообщением:",
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentBody { course_id, title }) => {
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
            teacher::ask_max_attempts(
                &bot,
                message.chat.id,
                course_id,
                title,
                text.to_owned(),
                &dialogue,
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            course_id,
            title,
            body,
            ..
        }) => {
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
            teacher::ask_grading(
                &bot,
                message.chat.id,
                DraftAssignment::new(course_id, title, body, max_attempts),
                &dialogue,
            )
            .await?;
        }
        BotState::Flow(FlowState::ChoosingGrading { draft, .. })
        | BotState::Flow(FlowState::AwaitingGradingText { draft }) => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
            let Some(grading) = parse_max_score(text) else {
                bot.send_message(
                    message.chat.id,
                    format!("Введите целое число от 1 до {MAX_SCORE}."),
                )
                .await?;
                return Ok(());
            };
            teacher::apply_grading(&bot, message.chat.id, draft, grading, &dialogue, &timezone)
                .await?;
        }
        BotState::Flow(FlowState::ChoosingLatePolicy { draft, .. })
        | BotState::Flow(FlowState::AwaitingLatePolicyText { draft }) => {
            let Some(text) = require_text(&bot, &message).await? else {
                return Ok(());
            };
            let Some(late_policy) = parse_late_policy(text) else {
                bot.send_message(
                    message.chat.id,
                    "Введите три числа через пробел, например «7 1 4».",
                )
                .await?;
                return Ok(());
            };
            teacher::apply_late_policy(
                &bot,
                message.chat.id,
                draft,
                Some(late_policy),
                &dialogue,
                &timezone,
            )
            .await?;
        }
        BotState::Flow(FlowState::ChoosingDeadline { target, .. })
        | BotState::Flow(FlowState::PickingDeadlineDate { target, .. })
        | BotState::Flow(FlowState::AwaitingDeadlineText { target }) => {
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
            teacher::apply_deadline(
                Screen {
                    bot: &bot,
                    chat_id: message.chat.id,
                    actor,
                    services: &services,
                    dialogue: &dialogue,
                    timezone: &timezone,
                    invites: &invites,
                },
                target,
                deadline_at,
            )
            .await?;
        }
        BotState::Flow(FlowState::ChoosingGradebookStudent { .. })
        | BotState::Flow(FlowState::ChoosingAttemptStudent { .. }) => {
            bot.send_message(message.chat.id, "Выберите студента кнопкой выше.")
                .await?;
        }
        BotState::Flow(FlowState::AwaitingAttemptAmount {
            assignment_id,
            student_id,
            ..
        }) => {
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
            teacher::grant_attempts(
                Screen {
                    bot: &bot,
                    chat_id: message.chat.id,
                    actor,
                    services: &services,
                    dialogue: &dialogue,
                    timezone: &timezone,
                    invites: &invites,
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
    deps: Deps<'_>,
) -> HandlerResult {
    let Deps {
        services,
        dialogue,
        timezone,
        invites,
    } = deps;
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
        invites,
    };

    match action {
        MenuAction::Assignments => student::show_assignments(screen).await,
        MenuAction::Archive => student::show_archive(screen).await,
        MenuAction::Queue => student::show_queue(screen).await,
        MenuAction::Review => review::show_next(screen).await,
        MenuAction::Courses => teacher::show_courses(screen).await,
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
    invites: BotInvites,
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
    let Some(action) = callbacks::parse(data) else {
        send_stale_button(&bot, chat_id).await?;
        return Ok(());
    };
    if action == CallbackAction::Noop {
        return Ok(());
    }
    if matches!(action, CallbackAction::Replayable(_)) {
        reset_dialogue(&dialogue).await?;
    }

    let Some(actor) = resolve_actor(&bot, chat_id, telegram_user_id, &services).await? else {
        return Ok(());
    };
    let screen = Screen {
        bot: &bot,
        chat_id,
        actor,
        services: &services,
        dialogue: &dialogue,
        timezone: &timezone,
        invites: &invites,
    };

    match action {
        CallbackAction::Replayable(action) => {
            handle_replayable_action(screen, action, &transfers).await
        }
        CallbackAction::Screen(action) => {
            handle_screen_action(screen, message.id, action, state, &transfers).await
        }
        CallbackAction::Flow(action) => {
            handle_flow_action(screen, message.id, action, state, &transfers).await
        }
        CallbackAction::Noop => Ok(()),
    }
}

async fn handle_replayable_action(
    screen: Screen<'_>,
    action: ReplayableAction,
    transfers: &super::FileTransfers,
) -> HandlerResult {
    match action {
        ReplayableAction::StudentAssignments => student::show_assignments(screen).await,
        ReplayableAction::StudentArchive => student::show_archive(screen).await,
        ReplayableAction::StudentQueue => student::show_queue(screen).await,
        ReplayableAction::Profile => show_profile(screen).await,
        ReplayableAction::Courses => teacher::show_courses(screen).await,
        ReplayableAction::NextReview => review::show_next(screen).await,
        ReplayableAction::Course(course_id) => {
            teacher::show_course_assignments(screen, course_id).await
        }
        ReplayableAction::Assignment(assignment_id) => {
            student::show_assignment(screen, assignment_id, transfers).await
        }
        ReplayableAction::ManageAssignment(assignment_id) => {
            teacher::show_assignment(screen, assignment_id).await
        }
        ReplayableAction::StartAssignmentCreation(course_id) => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingAssignmentTitle {
                    course_id,
                }))
                .await?;
            ask_input(screen.bot, screen.chat_id, "1/4. Введите название задания:").await
        }
        ReplayableAction::StartSubmission(assignment_id) => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingSubmission {
                    assignment_id,
                }))
                .await?;
            ask_input(
                screen.bot,
                screen.chat_id,
                "Пришлите ссылку или текст сдачи одним сообщением:",
            )
            .await
        }
        ReplayableAction::AssignmentFiles(assignment_id) => {
            assignment_files::show(
                screen.bot,
                screen.chat_id,
                screen.actor,
                assignment_id,
                false,
                screen.services,
                screen.dialogue,
            )
            .await
        }
        ReplayableAction::GrantAttempts(assignment_id) => {
            teacher::show_students(screen, assignment_id).await
        }
        ReplayableAction::GroupInvite(course_id) => {
            teacher::show_group_invite(screen, course_id).await
        }
        ReplayableAction::Gradebook(course_id) => teacher::show_gradebook(screen, course_id).await,
    }
}

async fn handle_screen_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: ScreenAction,
    state: BotState,
    transfers: &super::FileTransfers,
) -> HandlerResult {
    match action {
        ScreenAction::AddFile | ScreenAction::OpenFile(_) | ScreenAction::DeleteFile(_) => {
            assignment_files::handle_screen_action(
                assignment_files::CallbackContext {
                    bot: screen.bot,
                    chat_id: screen.chat_id,
                    message_id,
                    state: &state,
                    actor: screen.actor,
                    dialogue: screen.dialogue,
                    services: screen.services,
                    transfers,
                },
                action,
            )
            .await
        }
        ScreenAction::FinishFileCreation
        | ScreenAction::Publish(_)
        | ScreenAction::Archive(_)
        | ScreenAction::ChangeDeadline(_)
        | ScreenAction::RotateGroupInvite => {
            teacher::handle_screen_action(screen, message_id, action, state).await
        }
    }
}

async fn handle_flow_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: FlowAction,
    state: BotState,
    transfers: &super::FileTransfers,
) -> HandlerResult {
    match action {
        FlowAction::ConfirmFileDeletion | FlowAction::CancelFileDeletion => {
            assignment_files::handle_flow_action(
                assignment_files::CallbackContext {
                    bot: screen.bot,
                    chat_id: screen.chat_id,
                    message_id,
                    state: &state,
                    actor: screen.actor,
                    dialogue: screen.dialogue,
                    services: screen.services,
                    transfers,
                },
                action,
            )
            .await?;
        }
        FlowAction::Review {
            decision,
            submission_id: callback_submission_id,
        } => {
            let BotState::Flow(FlowState::AwaitingReviewDecision {
                submission_id,
                grading,
                penalty,
            }) = state
            else {
                return send_stale_button(screen.bot, screen.chat_id).await;
            };
            if submission_id != callback_submission_id {
                return send_stale_button(screen.bot, screen.chat_id).await;
            }

            // Балл несёт только принятие: отклонение спрашивает сразу комментарий.
            match (decision, grading.max_score()) {
                (ReviewDecision::Accept, Some(max_score)) => {
                    ask_review_score(
                        screen.bot,
                        screen.chat_id,
                        submission_id,
                        max_score,
                        penalty,
                        screen.dialogue,
                    )
                    .await?;
                }
                _ => {
                    ask_review_comment(
                        screen.bot,
                        screen.chat_id,
                        submission_id,
                        decision,
                        None,
                        screen.dialogue,
                    )
                    .await?;
                }
            }
        }
        FlowAction::Grading(choice) => {
            teacher::handle_grading(screen, message_id, choice, state).await?;
        }
        FlowAction::LatePolicy(choice) => {
            teacher::handle_late_policy(screen, message_id, choice, state).await?;
        }
        FlowAction::ReviewScore(choice) => {
            handle_review_score(screen, choice, state).await?;
        }
        FlowAction::MaxAttempts(choice) => {
            teacher::handle_max_attempts(screen, message_id, choice, state).await?;
        }
        FlowAction::Deadline(action) => {
            teacher::handle_deadline(screen, message_id, action, state).await?;
        }
        FlowAction::SelectStudent(student_id) => {
            teacher::select_student(screen, message_id, student_id, state).await?;
        }
        FlowAction::ExtraAttempts(choice) => {
            teacher::handle_extra_attempts(screen, message_id, choice, state).await?;
        }
        FlowAction::ConfirmInviteRotation | FlowAction::CancelInviteRotation => {
            teacher::handle_invite_rotation(screen, message_id, action, state).await?;
        }
    }

    Ok(())
}

async fn show_home(
    bot: &Bot,
    chat_id: ChatId,
    actor: TelegramUserId,
    services: &BotServices,
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
            bot.send_message(chat_id, GUEST_NEEDS_INVITE).await?;
        }
        Err(error) => send_app_error(bot, chat_id, error).await?,
    }

    Ok(())
}

async fn start_registration(
    bot: &Bot,
    chat_id: ChatId,
    telegram_user_id: TelegramUserId,
    payload: &str,
    services: &BotServices,
    dialogue: &BotDialogue,
    invites: &BotInvites,
) -> HandlerResult {
    match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(_) => return show_home(bot, chat_id, telegram_user_id, services).await,
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return send_app_error(bot, chat_id, error).await,
    }

    if payload.is_empty() {
        bot.send_message(chat_id, GUEST_NEEDS_INVITE).await?;
        return Ok(());
    }

    // Битая подпись и отозванная ссылка обязаны выглядеть для гостя одинаково.
    let Ok(invite) = invites.decode(payload) else {
        return send_error_code(bot, chat_id, ErrorCode::InviteInvalid).await;
    };
    let group = match services.users.resolve_group_invite(invite).await {
        Ok(group) => group,
        Err(error) => return send_app_error(bot, chat_id, error).await,
    };

    dialogue
        .update(BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
        }))
        .await?;
    ask_input(
        bot,
        chat_id,
        &format!(
            "Регистрация в группу {}.\n\nКак вас называть?",
            group.code().as_str()
        ),
    )
    .await
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

fn current_assignment_screen(
    assignment_id: AssignmentId,
    message_id: MessageId,
    state: &BotState,
) -> Option<AssignmentId> {
    let BotState::Screen(ScreenState::ManagingAssignment {
        message_id: current_message_id,
        assignment_id: current_assignment_id,
        ..
    }) = state
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

const SCORE_BUTTONS_LIMIT: u16 = 10;

async fn ask_review_score(
    bot: &Bot,
    chat_id: ChatId,
    submission_id: SubmissionId,
    max_score: u16,
    penalty: u16,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            penalty,
        }))
        .await?;

    if penalty > 0 {
        bot.send_message(
            chat_id,
            format!("⏳ Работа сдана с опозданием, штраф — {penalty}."),
        )
        .await?;
    }

    if max_score <= SCORE_BUTTONS_LIMIT {
        bot.send_message(chat_id, format!("Какой балл ставите из {max_score}?"))
            .reply_markup(review_score_keyboard(max_score))
            .await?;
        return Ok(());
    }

    ask_input(
        bot,
        chat_id,
        &format!("Какой балл ставите? Введите число от 0 до {max_score}:"),
    )
    .await
}

async fn ask_review_comment(
    bot: &Bot,
    chat_id: ChatId,
    submission_id: SubmissionId,
    decision: ReviewDecision,
    score: Option<u16>,
    dialogue: &BotDialogue,
) -> HandlerResult {
    dialogue
        .update(BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score,
        }))
        .await?;
    ask_input(bot, chat_id, "Напишите обязательный комментарий к решению:").await
}

async fn confirm_score(
    bot: &Bot,
    chat_id: ChatId,
    score: u16,
    penalty: u16,
    max_score: u16,
) -> HandlerResult {
    if penalty == 0 {
        return Ok(());
    }

    bot.send_message(
        chat_id,
        format!(
            "🏅 Итог: {score} − {penalty} = {} из {max_score}.",
            score.saturating_sub(penalty)
        ),
    )
    .await?;
    Ok(())
}

async fn handle_review_score(
    screen: Screen<'_>,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let BotState::Flow(FlowState::AwaitingReviewScore {
        submission_id,
        max_score,
        penalty,
    }) = state
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    let score = match choice {
        AmountChoice::Preset(score) if score <= max_score => score,
        AmountChoice::Preset(_) | AmountChoice::Manual => {
            return ask_input(
                screen.bot,
                screen.chat_id,
                &format!("Введите целое число от 0 до {max_score}:"),
            )
            .await;
        }
    };

    confirm_score(screen.bot, screen.chat_id, score, penalty, max_score).await?;
    ask_review_comment(
        screen.bot,
        screen.chat_id,
        submission_id,
        ReviewDecision::Accept,
        Some(score),
        screen.dialogue,
    )
    .await
}

fn parse_late_policy(text: &str) -> Option<LatePolicy> {
    let mut parts = text.split_whitespace();
    let mut next = || parse_number(parts.next()?);
    let (period_days, penalty, max_periods) = (next()?, next()?, next()?);

    parts
        .next()
        .is_none()
        .then(|| LatePolicy::new(period_days, penalty, max_periods).ok())
        .flatten()
}

fn parse_max_score(text: &str) -> Option<Grading> {
    Grading::scored(parse_number(text)?).ok()
}

fn parse_score(text: &str, max_score: u16) -> Option<u16> {
    let score = parse_number(text)?;

    (score <= max_score).then_some(score)
}

fn parse_number(text: &str) -> Option<u16> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    text.parse().ok()
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
    use chrono::{DateTime, FixedOffset, NaiveTime};
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::app::{ApplicationContainer, FilePolicy};
    use crate::bot::callbacks::{EXTRA_ATTEMPTS_PREFIX, MAX_ATTEMPTS_PREFIX};
    use crate::bot::views::{assignment_actions_keyboard, attempts_keyboard, review_keyboard};
    use crate::domain::{StudentGroupId, SubmissionId};

    use super::*;

    fn timezone() -> BotTimezone {
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
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

    #[test]
    fn every_button_the_bot_sends_is_routable() {
        use crate::bot::views::{
            assignment_files_keyboard, assignment_students_keyboard, attempts_keyboard,
            calendar_keyboard, confirm_file_deletion_keyboard, confirm_invite_rotation_keyboard,
            course_assignments_keyboard, deadline_options_keyboard, gradebook_students_keyboard,
            grading_keyboard, group_invite_keyboard, late_policy_keyboard, review_score_keyboard,
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
            review_keyboard(SubmissionId::new()),
            course_assignments_keyboard(course.id(), std::slice::from_ref(&assignment)),
            assignment_actions_keyboard(&assignment),
            attempts_keyboard(MAX_ATTEMPTS_PREFIX),
            attempts_keyboard(EXTRA_ATTEMPTS_PREFIX),
            grading_keyboard(),
            late_policy_keyboard(),
            review_score_keyboard(10),
            deadline_options_keyboard(&timezone),
            calendar_keyboard(today, today),
            assignment_files_keyboard(&assignment, &[file], true, true),
            assignment_files_keyboard(&assignment, &[], true, false),
            confirm_file_deletion_keyboard(),
            assignment_students_keyboard(assignment.id(), &[]),
            gradebook_students_keyboard(course.id(), &[]),
            group_invite_keyboard(course.id()),
            confirm_invite_rotation_keyboard(),
        ];

        for keyboard in keyboards {
            let json = serde_json::to_value(keyboard).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    let data = button["callback_data"].as_str().unwrap();
                    assert!(
                        callbacks::parse(data).is_some(),
                        "button {:?} sends {data}, which the callback parser rejects",
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
            let json = serde_json::to_value(attempts_keyboard(prefix)).unwrap();
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
            callbacks::parse(accept),
            Some(CallbackAction::Flow(FlowAction::Review {
                decision: crate::domain::ReviewDecision::Accept,
                submission_id,
            }))
        );
        assert_eq!(
            callbacks::parse(reject),
            Some(CallbackAction::Flow(FlowAction::Review {
                decision: crate::domain::ReviewDecision::Reject,
                submission_id,
            }))
        );
        assert!(accept.len() <= 64);
        assert!(reject.len() <= 64);
    }

    #[test]
    fn old_review_callback_identifies_a_different_submission() {
        let current = SubmissionId::new();
        let old = SubmissionId::new();
        let Some(CallbackAction::Flow(FlowAction::Review {
            submission_id: callback_submission,
            ..
        })) = callbacks::parse(&format!("review:accept:{}", old.as_uuid()))
        else {
            panic!("review callback must parse");
        };

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
        assert_eq!(student::queue_place(2, 5), "ваш номер в очереди: 2 из 5");
        assert_eq!(
            student::queue_place(0, 0),
            "преподаватель ещё не назначен на курс"
        );
    }

    #[test]
    fn assignment_action_requires_current_keyboard_and_assignment() {
        let current = AssignmentId::new();
        let other = AssignmentId::new();
        let state = BotState::Screen(ScreenState::ManagingAssignment {
            message_id: MessageId(10),
            assignment_id: current,
        });

        assert_eq!(
            current_assignment_screen(current, MessageId(10), &state),
            Some(current)
        );
        assert!(current_assignment_screen(current, MessageId(11), &state).is_none());
        assert!(current_assignment_screen(other, MessageId(10), &state).is_none());
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
            BotInvites::new(test_codec(), "hw_test_bot".to_owned()),
        )
        .await
        .unwrap();
    }

    fn test_codec() -> crate::invites::GroupInviteCodec {
        crate::invites::GroupInviteCodec::new(
            crate::invites::InviteSigningSecret::from_base64url(
                "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8",
            )
            .unwrap(),
        )
    }

    #[test]
    fn start_parses_with_and_without_an_invite_payload() {
        assert!(matches!(
            Command::parse("/start", "hw_test_bot"),
            Ok(Command::Start(payload)) if payload.is_empty()
        ));
        assert!(matches!(
            Command::parse("/start gi_payload", "hw_test_bot"),
            Ok(Command::Start(payload)) if payload == "gi_payload"
        ));
    }

    #[test]
    fn the_invite_link_carries_the_bot_username_from_get_me() {
        let invites = BotInvites::new(test_codec(), "hw_test_bot".to_owned());
        let group = crate::domain::StudentGroupId::new();
        let link = invites.link(group, crate::domain::InviteGeneration::FIRST);

        let payload = link
            .strip_prefix("https://t.me/hw_test_bot?start=")
            .unwrap();
        assert_eq!(invites.decode(payload).unwrap().group_id(), group);
    }
}
