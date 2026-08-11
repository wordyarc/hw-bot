use anyhow::{Context, anyhow};
use chrono::Utc;
use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::{self, Dialogue, InMemStorage};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, Message, User};
use teloxide::utils::command::BotCommands;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app::{
    Actor, AppError, ApplicationContainer, RegisterStudent, ReviewSubmission, SubmitHomework,
};
use crate::domain::{
    AssignmentId, AssignmentStatus, DomainError, ReviewDecision, Role, SubmissionId,
    SubmissionStatus, TelegramUserId,
};

const MESSAGE_LIMIT: usize = 4096;

type HandlerResult<T = ()> = anyhow::Result<T>;
type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    application: ApplicationContainer,
}

type BotServices = ApplicationContainer;

#[derive(Clone, Default)]
enum BotState {
    #[default]
    Idle,
    AwaitingRegistrationName,
    AwaitingRegistrationGroup {
        display_name: String,
        group_codes: Vec<String>,
    },
    AwaitingSubmission {
        assignment_id: AssignmentId,
    },
    AwaitingReviewDecision {
        submission_id: SubmissionId,
    },
    AwaitingReviewComment {
        submission_id: SubmissionId,
        decision: ReviewDecision,
    },
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
enum Command {
    #[command(description = "открыть главное меню")]
    Start,
    #[command(description = "открыть главное меню")]
    Menu,
    #[command(description = "отменить текущее действие")]
    Cancel,
    #[command(description = "показать помощь")]
    Help,
}

impl TelegramBot {
    pub fn new(
        application: ApplicationContainer,
        token: impl Into<String>,
        api_base_url: &str,
    ) -> anyhow::Result<Self> {
        let api_url = api_base_url
            .parse()
            .with_context(|| format!("invalid Telegram API base URL: {api_base_url}"))?;
        let bot = Bot::new(token).set_api_url(api_url);

        Ok(Self { bot, application })
    }

    pub async fn run(self, cancellation: CancellationToken) {
        let storage = InMemStorage::<BotState>::new();
        let mut dispatcher = Dispatcher::builder(self.bot, schema())
            .dependencies(dptree::deps![storage, self.application])
            .build();
        let shutdown = dispatcher.shutdown_token();
        let dispatch = dispatcher.dispatch();
        tokio::pin!(dispatch);

        tokio::select! {
            () = &mut dispatch => {},
            () = cancellation.cancelled() => {
                let _ = shutdown.shutdown();
                dispatch.await;
            }
        }
    }
}

fn schema() -> UpdateHandler<anyhow::Error> {
    let messages = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(dptree::endpoint(handle_message));
    let callbacks = Update::filter_callback_query().endpoint(handle_callback);

    dialogue::enter::<Update, InMemStorage<BotState>, BotState, _>()
        .branch(messages)
        .branch(callbacks)
}

async fn handle_command(
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

async fn handle_message(
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
                Err(error @ AppError::Domain(DomainError::SubmissionIsNotPendingReview)) => {
                    dialogue.exit().await?;
                    send_app_error(&bot, message.chat.id, error).await?;
                }
                Err(error) => send_app_error(&bot, message.chat.id, error).await?,
            }
        }
    }

    Ok(())
}

async fn handle_callback(
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
        "menu:courses" => show_courses(&bot, chat_id, actor, &services).await?,
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
        Err(AppError::NotFound { .. }) => {
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
) -> HandlerResult {
    let courses = match services.courses.list_for_teacher(actor).await {
        Ok(courses) => courses,
        Err(error) => {
            send_app_error(bot, chat_id, error).await?;
            return Ok(());
        }
    };
    if courses.is_empty() {
        bot.send_message(chat_id, "У вас пока нет учебных курсов.")
            .await?;
        return Ok(());
    }

    let mut text = String::from("Ваши курсы:\n");
    for course in courses {
        text.push_str(&format!(
            "\n{} · {}\n",
            course.subject.name().as_str(),
            course.group.code().as_str(),
        ));
    }
    send_message_chunks(bot, chat_id, text, None).await?;
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

fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Мои задания",
        "menu:assignments",
    )]])
}

fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || "Проверить следующую".to_owned(),
        |pending| format!("Проверить следующую ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(review_label, "review:next")],
        vec![InlineKeyboardButton::callback("Мои курсы", "menu:courses")],
    ])
}

fn group_keyboard(group_codes: &[String]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        group_codes
            .iter()
            .enumerate()
            .map(|(index, code)| {
                vec![InlineKeyboardButton::callback(
                    code.clone(),
                    format!("register_group:{index}"),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "Принять",
            format!("review:accept:{}", submission_id.as_uuid()),
        ),
        InlineKeyboardButton::callback(
            "Отклонить",
            format!("review:reject:{}", submission_id.as_uuid()),
        ),
    ]])
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

fn format_deadline(deadline: chrono::DateTime<Utc>) -> String {
    deadline.format("%d.%m.%Y %H:%M UTC").to_string()
}

fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "на проверке",
        SubmissionStatus::Accepted => "принята",
        SubmissionStatus::Rejected => "отклонена",
    }
}

fn button_label(label: &str) -> String {
    let truncated = label.chars().count() > 60;
    let mut label: String = label.chars().take(60).collect();
    if truncated {
        label.push_str("...");
    }
    label
}

fn message_chunks(text: String) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;

    for character in text.chars() {
        if current_len == MESSAGE_LIMIT {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push(character);
        current_len += 1;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    let chunks = message_chunks(text);
    let last = chunks.len().saturating_sub(1);

    for (index, chunk) in chunks.into_iter().enumerate() {
        if index == last
            && let Some(keyboard) = keyboard.clone()
        {
            bot.send_message(chat_id, chunk)
                .reply_markup(keyboard)
                .await?;
        } else {
            bot.send_message(chat_id, chunk).await?;
        }
    }

    Ok(())
}

async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, "Эта кнопка устарела. Откройте /menu ещё раз.")
        .await?;
    Ok(())
}

async fn send_app_error(bot: &Bot, chat_id: ChatId, error: AppError) -> HandlerResult {
    if matches!(error, AppError::Db(_)) {
        tracing::error!(error = ?error, "Telegram command failed");
    }
    bot.send_message(chat_id, app_error_message(&error)).await?;
    Ok(())
}

fn app_error_message(error: &AppError) -> &'static str {
    match error {
        AppError::NotFound { .. } => "Запрошенные данные не найдены.",
        AppError::Conflict { .. } | AppError::AssignmentChangedConcurrently => {
            "Данные уже изменились. Повторите действие."
        }
        AppError::Db(_) => "Произошла внутренняя ошибка. Попробуйте позже.",
        AppError::Domain(error) => match error {
            DomainError::EmptyField { .. } => "Сообщение не должно быть пустым.",
            DomainError::AssignmentIsNotPublished => "Это задание сейчас не принимает работы.",
            DomainError::AssignmentDeadlineExceeded { .. } => "Дедлайн задания уже прошёл.",
            DomainError::AttemptsExhausted { .. } => "Доступные попытки закончились.",
            DomainError::SubmissionAlreadyAccepted => "Работа уже принята.",
            DomainError::SubmissionAlreadyPendingReview => "Предыдущая работа ещё на проверке.",
            DomainError::SubmissionIsNotPendingReview => {
                "Эту сдачу уже проверил другой преподаватель."
            }
            DomainError::UserAlreadyRegistered => "Вы уже зарегистрированы.",
            DomainError::PermissionDenied { .. }
            | DomainError::StudentDoesNotBelongToAssignmentGroup => "Это действие вам недоступно.",
            _ => "Не удалось выполнить действие. Проверьте данные и попробуйте ещё раз.",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::postgres::PgPoolOptions;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
    fn long_messages_are_split_without_losing_content() {
        let source = "я".repeat(MESSAGE_LIMIT * 2 + 1);
        let chunks = message_chunks(source.clone());

        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MESSAGE_LIMIT)
        );
        assert_eq!(chunks.concat(), source);
    }

    #[test]
    fn common_submission_errors_have_student_facing_text() {
        assert_eq!(
            app_error_message(&DomainError::SubmissionAlreadyPendingReview.into()),
            "Предыдущая работа ещё на проверке."
        );
        assert_eq!(
            app_error_message(&DomainError::SubmissionIsNotPendingReview.into()),
            "Эту сдачу уже проверил другой преподаватель."
        );
    }

    #[test]
    fn group_keyboard_keeps_callback_data_short() {
        let keyboard = group_keyboard(&["б9122-02.03.21".to_owned(), "магистратура-2".to_owned()]);
        let json = serde_json::to_value(keyboard).unwrap();

        assert_eq!(
            json["inline_keyboard"][0][0]["callback_data"],
            "register_group:0"
        );
        assert_eq!(
            json["inline_keyboard"][1][0]["callback_data"],
            "register_group:1"
        );
    }

    #[test]
    fn button_labels_are_truncated_only_when_needed() {
        assert_eq!(button_label("Мои задания"), "Мои задания");
        assert_eq!(button_label(&"я".repeat(61)).chars().count(), 63);
        assert!(button_label(&"я".repeat(61)).ends_with("..."));
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
