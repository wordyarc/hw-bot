use super::Bot;
use crate::app::Actor;
use crate::domain::TelegramUserId;
use anyhow::{Context, anyhow};
use teloxide::dispatching::dialogue::InMemStorageError;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::types::{CallbackQuery, ChatId, Message, MessageId, User};
use teloxide::utils::command::BotCommands;

use super::assignment_files;
use super::callbacks::{self, CallbackAction, FlowAction, ReplayableAction, ScreenAction};
use super::context::{Deps, Screen, TextStep};
use super::errors::send_app_error;
use super::invites::BotInvites;
use super::state::{BotDialogue, BotState, FlowState, ScreenState};
use super::texts;
use super::timezone::BotTimezone;
use super::views::{MenuAction, ask_input, menu_action, send_stale_button};
use super::{BotServices, HandlerResult};

pub(super) mod home;
mod input;
mod registration;
mod review;
mod student;
mod teacher;

use home::{send_help, show_home, show_profile};
use registration::start_registration;

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
    tracing::Span::current().record(
        "action",
        match &command {
            Command::Start(_) => "start",
            Command::Menu => "menu",
            Command::Tasks => "tasks",
            Command::Queue => "queue",
            Command::Archive => "archive",
            Command::Review => "review",
            Command::Courses => "courses",
            Command::Profile => "profile",
            Command::Cancel => "cancel",
            Command::Help => "help",
        },
    );
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

#[expect(
    clippy::too_many_arguments,
    reason = "Зависимости endpoint инъецирует Teloxide"
)]
pub(super) async fn handle_message(
    bot: Bot,
    message: Message,
    dialogue: BotDialogue,
    state: BotState,
    services: BotServices,
    transfers: super::FileTransfers,
    timezone: BotTimezone,
    invites: BotInvites,
) -> HandlerResult {
    tracing::Span::current().record("action", "input");
    if !ensure_private_message(&bot, &message).await? {
        return Ok(());
    }

    let actor = actor_from_message(&message)?;
    let deps = Deps {
        services: &services,
        dialogue: &dialogue,
        timezone: &timezone,
        invites: &invites,
    };
    let step = TextStep {
        bot: &bot,
        message: &message,
        telegram_user_id: actor,
        deps,
    };

    if let Some(action) = message.text().and_then(menu_action) {
        reset_dialogue(&dialogue).await?;
        return run_menu_action(&bot, message.chat.id, actor, action, deps).await;
    }

    match state {
        BotState::Idle => {
            bot.send_message(message.chat.id, texts::USE_THE_MENU)
                .await?;
        }
        BotState::Flow(FlowState::AwaitingRegistrationName { invite }) => {
            registration::submit_name(step, invite).await?;
        }
        BotState::Flow(FlowState::AwaitingSubmission { assignment_id }) => {
            student::submit_homework_text(step, assignment_id).await?;
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
            review::submit_score_text(step, submission_id, max_score, penalty).await?;
        }
        BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id,
            decision,
            score,
        }) => {
            review::submit_comment(step, submission_id, decision, score).await?;
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
        BotState::Screen(ScreenState::ManagingAssignmentFiles { .. }) => {
            bot.send_message(
                message.chat.id,
                "Выберите действие кнопкой под списком вложений.",
            )
            .await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentFile {
            assignment_id,
            expected_revision,
            creation_flow,
        }) => {
            assignment_files::submit_file(
                step,
                assignment_id,
                expected_revision,
                creation_flow,
                &transfers,
            )
            .await?;
        }
        BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. }) => {
            bot.send_message(message.chat.id, "Подтвердите удаление кнопкой выше.")
                .await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id }) => {
            teacher::submit_title(step, course_id).await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentBody { course_id, title }) => {
            teacher::submit_body(step, course_id, title).await?;
        }
        BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            course_id,
            title,
            body,
            ..
        }) => {
            teacher::submit_max_attempts(step, course_id, title, body).await?;
        }
        BotState::Flow(FlowState::ChoosingGrading { draft, .. })
        | BotState::Flow(FlowState::AwaitingGradingText { draft }) => {
            teacher::submit_grading_text(step, draft).await?;
        }
        BotState::Flow(FlowState::ChoosingLatePolicy { draft, .. })
        | BotState::Flow(FlowState::AwaitingLatePolicyText { draft }) => {
            teacher::submit_late_policy_text(step, draft).await?;
        }
        BotState::Flow(FlowState::ChoosingDeadline { target, .. })
        | BotState::Flow(FlowState::AwaitingDeadlineText { target }) => {
            teacher::submit_deadline_text(step, target).await?;
        }
        BotState::Flow(FlowState::ChoosingStudent { .. }) => {
            bot.send_message(message.chat.id, "Выберите студента кнопкой выше.")
                .await?;
        }
        BotState::Flow(FlowState::AwaitingAttemptAmount {
            assignment_id,
            student_id,
            ..
        }) => {
            teacher::submit_attempt_amount(step, assignment_id, student_id).await?;
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
    tracing::Span::current().record("action", tracing::field::debug(action));
    if action == MenuAction::Help {
        return send_help(bot, chat_id, telegram_user_id, deps.services, deps.timezone).await;
    }

    let Some(actor) = resolve_actor(bot, chat_id, telegram_user_id, deps.services).await? else {
        return Ok(());
    };
    let screen = Screen::new(bot, chat_id, actor, deps);

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

#[expect(
    clippy::too_many_arguments,
    reason = "Зависимости endpoint инъецирует Teloxide"
)]
pub(super) async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    dialogue: BotDialogue,
    state: BotState,
    services: BotServices,
    transfers: super::FileTransfers,
    timezone: BotTimezone,
    invites: BotInvites,
) -> HandlerResult {
    let action = query.data.as_deref().and_then(callbacks::parse);
    if let Some(action) = action {
        tracing::Span::current().record("action", tracing::field::debug(action));
    }
    if let Err(error) = bot
        .answer_callback_query(query.id.clone())
        .cache_time(0)
        .await
    {
        if matches!(
            error,
            teloxide::RequestError::Api(teloxide::ApiError::InvalidToken)
        ) {
            return Err(error.into());
        }
        tracing::warn!(
            kind = "callback_ack_failed",
            "Telegram callback acknowledgement failed"
        );
    }

    let Some(message) = query.regular_message() else {
        return Ok(());
    };
    if !message.chat.is_private() {
        return Ok(());
    }
    let chat_id = message.chat.id;
    let telegram_user_id = actor_from_user(&query.from)?;
    if query.data.is_none() {
        return Ok(());
    }
    let Some(action) = action else {
        send_stale_button(&bot, chat_id).await?;
        return Ok(());
    };
    if action == CallbackAction::Noop {
        return Ok(());
    }
    if matches!(action, CallbackAction::Replayable(_)) {
        reset_dialogue(&dialogue).await?;
    }

    let deps = Deps {
        services: &services,
        dialogue: &dialogue,
        timezone: &timezone,
        invites: &invites,
    };
    let Some(actor) = resolve_actor(&bot, chat_id, telegram_user_id, &services).await? else {
        return Ok(());
    };
    let screen = Screen::new(&bot, chat_id, actor, deps);

    match action {
        CallbackAction::Replayable(action) => {
            handle_replayable_action(screen, action, &transfers).await
        }
        CallbackAction::Screen(action) => {
            handle_screen_action(screen, message.id, action, state, &transfers).await
        }
        CallbackAction::Flow(action) => handle_flow_action(screen, message.id, action, state).await,
        CallbackAction::Noop => Ok(()),
    }
}

async fn handle_replayable_action(
    screen: Screen<'_>,
    action: ReplayableAction,
    transfers: &super::FileTransfers,
) -> HandlerResult {
    match action {
        ReplayableAction::Page(target, page) => {
            use super::callbacks::PageTarget;
            let page = crate::pagination::PageRequest::new(page);
            match target {
                PageTarget::Assignments => student::show_assignments_page(screen, page).await,
                PageTarget::Archive => student::show_archive_page(screen, page).await,
                PageTarget::Queue => student::show_queue_page(screen, page).await,
                PageTarget::Courses => teacher::show_courses_page(screen, page).await,
                PageTarget::Course(id) => {
                    teacher::show_course_assignments_page(screen, id, page).await
                }
                PageTarget::Students(id) => teacher::show_students_page(screen, id, page).await,
                PageTarget::Gradebook(id) => teacher::show_gradebook_page(screen, id, page).await,
                PageTarget::History(id) => student::show_history(screen, id, page).await,
                PageTarget::StudentGrades(course, student) => {
                    teacher::show_student_grades_page(screen, course, student, page).await
                }
            }
        }
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
            ask_input(screen.bot, screen.chat_id, texts::SUBMISSION_PROMPT).await
        }
        ReplayableAction::AssignmentFiles(assignment_id) => {
            assignment_files::show(screen, assignment_id, false).await
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
        ScreenAction::AddFile => assignment_files::ask_upload(screen, message_id, &state).await,
        ScreenAction::OpenFile(index) => {
            assignment_files::open(screen, message_id, &state, index, transfers).await
        }
        ScreenAction::DeleteFile(index) => {
            assignment_files::request_deletion(screen, message_id, &state, index).await
        }
        ScreenAction::FinishFileCreation => {
            let Some((assignment_id, _, true)) = state.managing_assignment_files(message_id) else {
                return send_stale_button(screen.bot, screen.chat_id).await;
            };
            teacher::show_assignment(screen, assignment_id).await
        }
        ScreenAction::Publish(id) => {
            teacher::publish_assignment(screen, message_id, id, &state).await
        }
        ScreenAction::Archive(id) => {
            teacher::archive_assignment(screen, message_id, id, &state).await
        }
        ScreenAction::ChangeDeadline(id) => {
            teacher::ask_assignment_deadline(screen, message_id, id, &state).await
        }
        ScreenAction::RotateGroupInvite => {
            teacher::ask_invite_rotation(screen, message_id, &state).await
        }
    }
}

async fn handle_flow_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: FlowAction,
    state: BotState,
) -> HandlerResult {
    match action {
        FlowAction::ConfirmFileDeletion => {
            assignment_files::confirm_deletion(screen, message_id, &state).await?;
        }
        FlowAction::CancelFileDeletion => {
            assignment_files::cancel_deletion(screen, message_id, &state).await?;
        }
        FlowAction::Review {
            decision,
            submission_id,
        } => {
            review::handle_decision(screen, submission_id, decision, state).await?;
        }
        FlowAction::Grading(choice) => {
            teacher::handle_grading(screen, message_id, choice, state).await?;
        }
        FlowAction::LatePolicy(choice) => {
            teacher::handle_late_policy(screen, message_id, choice, state).await?;
        }
        FlowAction::ReviewScore {
            submission_id,
            choice,
        } => {
            review::handle_review_score(screen, submission_id, choice, state).await?;
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
        FlowAction::ConfirmInviteRotation => {
            teacher::confirm_invite_rotation(screen, message_id, &state).await?;
        }
        FlowAction::CancelInviteRotation => {
            teacher::cancel_invite_rotation(screen, message_id, &state).await?;
        }
    }

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

pub(super) async fn resolve_actor(
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
    use super::input::parse_positive_attempts;
    use crate::bot::callbacks::{EXTRA_ATTEMPTS_PREFIX, MAX_ATTEMPTS_PREFIX};
    use crate::bot::views::{
        assignment_actions_keyboard, attempts_keyboard, review_keyboard, student_menu, teacher_menu,
    };
    use crate::domain::SubmissionId;
    use chrono::{FixedOffset, NaiveTime, Utc};

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
            review_score_keyboard(SubmissionId::new(), 10),
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
}
