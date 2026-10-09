use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use teloxide::dispatching::dialogue::InMemStorageError;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::types::{CallbackQuery, Message, MessageId, User};
use teloxide::utils::command::BotCommands;

use super::Bot;
use super::HandlerResult;
use super::assignment_files;
use super::callbacks::{
    self, CallbackAction, FlowAction, InformationalAction, PageTarget, ReplayableAction,
    ScreenAction,
};
use super::context::{BotContext, Chat, Deps, Screen, TextStep};
use super::errors::finish_handler;
use super::state::{BotDialogue, BotState, FlowState, ScreenState};
use super::texts;
use super::views::{MenuAction, menu_action, send_stale_button};
use crate::domain::{Role, TelegramUserId};
use crate::pagination::PageRequest;
use crate::telegram::is_invalid_token;

mod contact;
pub(super) mod home;
pub(super) mod input;
mod registration;
mod review;
mod sandbox;
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
    #[command(description = "📘 мои предметы")]
    Courses,
    #[command(description = "👤 профиль")]
    Profile,
    #[command(description = "❌ отменить текущее действие")]
    Cancel,
    #[command(description = "❓ помощь")]
    Help,
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Start(_) => "start",
            Self::Menu => "menu",
            Self::Tasks => "tasks",
            Self::Queue => "queue",
            Self::Archive => "archive",
            Self::Review => "review",
            Self::Courses => "courses",
            Self::Profile => "profile",
            Self::Cancel => "cancel",
            Self::Help => "help",
        }
    }

    fn visible_to(&self, role: Option<Role>) -> bool {
        match self {
            Self::Start(_) => role.is_none(),
            Self::Help => true,
            Self::Menu | Self::Profile | Self::Cancel => role.is_some(),
            Self::Tasks | Self::Queue | Self::Archive => matches!(role, Some(Role::Student { .. })),
            Self::Review | Self::Courses => matches!(role, Some(Role::Teacher | Role::Admin)),
        }
    }
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
    state: BotState,
    context: BotContext,
) -> HandlerResult {
    let deps = Deps {
        context: &context,
        dialogue: &dialogue,
    };
    let chat_id = message.chat.id;
    let chat = Chat::new(&bot, chat_id, deps);
    let result = async {
        tracing::Span::current().record("action", command.name());
        if !ensure_private_message(&bot, &message).await? {
            return Ok(());
        }

        let telegram_user_id = message_sender_id(&message)?;

        if let Some(form) = state.active_terms()
            && !matches!(command, Command::Help)
        {
            if matches!(command, Command::Cancel) {
                let Some(screen) = chat.resolve(telegram_user_id).await? else {
                    return Ok(());
                };
                if let BotState::Flow(FlowState::ConfirmingTermsExit { message_id, .. }) = &state {
                    return teacher::handle_terms(
                        screen,
                        *message_id,
                        callbacks::TermsAction::Stay,
                        state,
                    )
                    .await;
                }
                return teacher::back(screen, form.clone()).await;
            }
            reset_dialogue(&dialogue).await?;
            teacher::abandon(&bot, chat_id, form).await?;
        }
        if let Some(form) = state.active_makeup()
            && !matches!(command, Command::Help)
        {
            if matches!(command, Command::Cancel) {
                let Some(screen) = chat.resolve(telegram_user_id).await? else {
                    return Ok(());
                };
                return teacher::makeup_back(screen, form.clone()).await;
            }
            reset_dialogue(&dialogue).await?;
            teacher::abandon_makeup(&bot, chat_id, form).await?;
        }

        if let Some(action) = menu_action_for(&command) {
            reset_dialogue(&dialogue).await?;
            return run_menu_action(chat, telegram_user_id, action).await;
        }
        match command {
            Command::Start(payload) => {
                reset_dialogue(&dialogue).await?;
                start_registration(chat, telegram_user_id, payload.trim()).await
            }
            Command::Menu => {
                reset_dialogue(&dialogue).await?;
                show_home(chat, telegram_user_id).await
            }
            Command::Cancel => {
                reset_dialogue(&dialogue).await?;
                bot.send_message(chat_id, texts::ACTION_CANCELLED).await?;
                show_home(chat, telegram_user_id).await
            }
            Command::Help => send_help(chat, telegram_user_id).await,
            _ => Ok(()),
        }
    }
    .await;
    finish_handler(&bot, chat_id, result).await
}

fn menu_action_for(command: &Command) -> Option<MenuAction> {
    match command {
        Command::Tasks => Some(MenuAction::Assignments),
        Command::Queue => Some(MenuAction::Queue),
        Command::Archive => Some(MenuAction::Archive),
        Command::Review => Some(MenuAction::Review),
        Command::Courses => Some(MenuAction::Courses),
        Command::Profile => Some(MenuAction::Profile),
        Command::Start(_) | Command::Menu | Command::Cancel | Command::Help => None,
    }
}

pub(super) async fn handle_message(
    bot: Bot,
    message: Message,
    dialogue: BotDialogue,
    state: BotState,
    context: BotContext,
) -> HandlerResult {
    let chat_id = message.chat.id;
    let result = async {
        tracing::Span::current().record("action", "input");
        if !ensure_private_message(&bot, &message).await? {
            return Ok(());
        }

        let telegram_user_id = message_sender_id(&message)?;
        let deps = Deps {
            context: &context,
            dialogue: &dialogue,
        };
        let step = TextStep::new(&bot, &message, telegram_user_id, deps);

        if let Some(action) = message.text().and_then(menu_action) {
            if let Some(form) = state.active_terms() {
                let Some(screen) = step.resolve().await? else {
                    return Ok(());
                };
                return teacher::confirm_exit(screen, form.clone(), action, telegram_user_id).await;
            }
            reset_dialogue(&dialogue).await?;
            if let Some(form) = state.active_makeup() {
                teacher::abandon_makeup(&bot, chat_id, form).await?;
            }
            return run_menu_action(step.chat(), telegram_user_id, action).await;
        }

        match state {
            BotState::Idle => {
                if step.resolve().await?.is_some() {
                    bot.send_message(message.chat.id, texts::USE_THE_MENU)
                        .await?;
                }
            }
            BotState::Screen(ScreenState::Guest) => {
                home::send_guest_hint(&bot, message.chat.id).await?;
            }
            BotState::Flow(FlowState::AwaitingRegistrationName { invite }) => {
                registration::submit_name(step, invite).await?;
            }
            BotState::Flow(FlowState::AwaitingSubmission {
                assignment_id,
                makeup,
            }) => {
                student::submit_homework_text(step, assignment_id, makeup).await?;
            }
            BotState::Flow(FlowState::AwaitingReviewScore {
                submission_id,
                max_score,
                late,
            }) => {
                review::submit_score_text(step, submission_id, max_score, late).await?;
            }
            BotState::Flow(FlowState::AwaitingReviewComment {
                submission_id,
                decision,
                score,
            }) => {
                review::submit_comment(step, submission_id, decision, score).await?;
            }
            BotState::Flow(FlowState::AwaitingBulkReviewComment { decision, cutoff }) => {
                sandbox::submit_comment(step, decision, cutoff).await?;
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
                )
                .await?;
            }
            BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id }) => {
                teacher::submit_title(step, course_id).await?;
            }
            BotState::Flow(FlowState::AwaitingAssignmentBody { course_id, title }) => {
                teacher::submit_body(step, course_id, title).await?;
            }
            BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts { text, .. }) => {
                teacher::submit_max_attempts(step, text).await?;
            }
            BotState::Flow(FlowState::ChoosingGrading { draft, .. })
            | BotState::Flow(FlowState::AwaitingGradingText { draft }) => {
                teacher::submit_grading_text(step, draft).await?;
            }
            BotState::Flow(FlowState::ConfiguringTerms(form)) => {
                teacher::submit_terms(step, form).await?;
            }
            BotState::Flow(FlowState::ConfiguringMakeup(form)) => {
                teacher::submit_makeup_text(step, form).await?;
            }
            BotState::Flow(FlowState::AwaitingAttemptAmount {
                assignment_id,
                student_id,
                ..
            }) => {
                teacher::submit_attempt_amount(step, assignment_id, student_id).await?;
            }
            state => {
                if let Some(hint) = button_hint(&state) {
                    bot.send_message(chat_id, hint).await?;
                }
            }
        }

        Ok(())
    }
    .await;
    finish_handler(&bot, chat_id, result).await
}

async fn run_menu_action(
    chat: Chat<'_>,
    telegram_user_id: TelegramUserId,
    action: MenuAction,
) -> HandlerResult {
    tracing::Span::current().record("action", tracing::field::debug(action));
    let Some(target) = menu_target(action) else {
        return send_help(chat, telegram_user_id).await;
    };

    let Some(screen) = chat.resolve(telegram_user_id).await? else {
        return Ok(());
    };

    handle_replayable_action(screen, target, Utc::now()).await
}

fn menu_target(action: MenuAction) -> Option<ReplayableAction> {
    Some(match action {
        MenuAction::Assignments => ReplayableAction::Page(PageTarget::Assignments, 0),
        MenuAction::Archive => ReplayableAction::Page(PageTarget::Archive, 0),
        MenuAction::Queue => ReplayableAction::Page(PageTarget::Queue, 0),
        MenuAction::Review => ReplayableAction::ReviewHub,
        MenuAction::Courses => ReplayableAction::Page(PageTarget::Courses, 0),
        MenuAction::Sandbox => ReplayableAction::Page(PageTarget::Sandbox, 0),
        MenuAction::Profile => ReplayableAction::Profile,
        MenuAction::Help => return None,
    })
}

pub(super) async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    dialogue: BotDialogue,
    state: BotState,
    context: BotContext,
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
        if is_invalid_token(&error) {
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
    let result = async {
        let telegram_user_id = sender_id(&query.from)?;
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
            if let Some(form) = state.active_terms() {
                teacher::abandon(&bot, chat_id, form).await?;
            }
            if let Some(form) = state.active_makeup() {
                teacher::abandon_makeup(&bot, chat_id, form).await?;
            }
        }

        let deps = Deps {
            context: &context,
            dialogue: &dialogue,
        };
        let chat = Chat::new(&bot, chat_id, deps);
        let Some(screen) = chat.resolve(telegram_user_id).await? else {
            return Ok(());
        };

        match action {
            CallbackAction::Replayable(action) => {
                handle_replayable_action(screen, action, message.date).await
            }
            CallbackAction::Screen(action) => {
                handle_screen_action(screen, message.id, action, state).await
            }
            CallbackAction::Flow(action) => {
                handle_flow_action(screen, telegram_user_id, message.id, action, state).await
            }
            CallbackAction::Informational(InformationalAction::StudentContact {
                course_id,
                student_id,
            }) => contact::show(screen, message.id, course_id, student_id).await,
            CallbackAction::Noop => Ok(()),
        }
    }
    .await;
    finish_handler(&bot, chat_id, result).await
}

async fn handle_replayable_action(
    screen: Screen<'_>,
    action: ReplayableAction,
    shown_at: DateTime<Utc>,
) -> HandlerResult {
    match action {
        ReplayableAction::Page(target, page) => {
            let page = PageRequest::new(page);
            match target {
                PageTarget::Assignments => student::show_assignments_page(screen, page).await,
                PageTarget::Archive => student::show_archive_page(screen, page).await,
                PageTarget::Queue => student::show_queue_page(screen, page).await,
                PageTarget::Courses => teacher::show_courses_page(screen, page).await,
                PageTarget::Sandbox => sandbox::show_page(screen, page).await,
                PageTarget::Course(id) => {
                    teacher::show_course_assignments_page(screen, id, page).await
                }
                PageTarget::Students(id) => teacher::show_students_page(screen, id, page).await,
                PageTarget::Gradebook(id) => teacher::show_gradebook_page(screen, id, page).await,
                PageTarget::History(id) => student::show_history(screen, id, page).await,
                PageTarget::StudentGrades(course, student) => {
                    teacher::show_student_grades_page(screen, course, student, page, None).await
                }
            }
        }
        ReplayableAction::Profile => show_profile(screen).await,
        ReplayableAction::ReviewHub => review::show_hub(screen).await,
        ReplayableAction::NextReview => review::show_next(screen).await,
        ReplayableAction::NextMakeupReview => review::show_next_makeup(screen).await,
        ReplayableAction::Course(course_id) => {
            teacher::show_course_assignments_page(screen, course_id, PageRequest::default()).await
        }
        ReplayableAction::CourseMakeup(course_id) => teacher::open_makeup(screen, course_id).await,
        ReplayableAction::Assignment(assignment_id) => {
            student::show_assignment(screen, assignment_id).await
        }
        ReplayableAction::ManageAssignment(assignment_id) => {
            teacher::show_assignment(screen, assignment_id).await
        }
        ReplayableAction::StartAssignmentCreation(course_id) => {
            screen
                .dialogue()
                .update(BotState::Flow(FlowState::AwaitingAssignmentTitle {
                    course_id,
                }))
                .await?;
            screen.ask(texts::ASSIGNMENT_TITLE_PROMPT).await
        }
        ReplayableAction::StartSubmission(assignment_id) => {
            student::start_submission(screen, assignment_id, shown_at).await
        }
        ReplayableAction::AssignmentFiles(assignment_id) => {
            assignment_files::show(screen, assignment_id, false).await
        }
        ReplayableAction::GrantAttempts(assignment_id) => {
            teacher::show_students_page(screen, assignment_id, PageRequest::default()).await
        }
        ReplayableAction::GroupInvite(course_id) => {
            teacher::show_group_invite(screen, course_id).await
        }
        ReplayableAction::Gradebook(course_id) => {
            teacher::show_gradebook_page(screen, course_id, PageRequest::default()).await
        }
        ReplayableAction::ReviewSubmission(submission_id) => {
            review::open(screen, submission_id).await
        }
    }
}

async fn handle_screen_action(
    screen: Screen<'_>,
    message_id: MessageId,
    action: ScreenAction,
    state: BotState,
) -> HandlerResult {
    match action {
        ScreenAction::AddFile => assignment_files::ask_upload(screen, message_id, &state).await,
        ScreenAction::OpenFile(index) => {
            assignment_files::open(screen, message_id, &state, index).await
        }
        ScreenAction::DeleteFile(index) => {
            assignment_files::request_deletion(screen, message_id, &state, index).await
        }
        ScreenAction::FinishFileCreation => {
            let Some(files) = state
                .files_screen(message_id)
                .filter(|files| files.creation_flow)
            else {
                return screen.stale().await;
            };
            teacher::show_assignment(screen, files.assignment_id).await
        }
        ScreenAction::Publish(id) => {
            teacher::publish_assignment(screen, message_id, id, &state).await
        }
        ScreenAction::Archive(id) => {
            teacher::archive_assignment(screen, message_id, id, &state).await
        }
        ScreenAction::ChangeDeadline(id) => {
            teacher::begin_terms_edit(screen, message_id, id, &state).await
        }
        ScreenAction::RotateGroupInvite => {
            teacher::ask_invite_rotation(screen, message_id, &state).await
        }
        ScreenAction::BulkReview(decision) => {
            sandbox::ask_comment(screen, message_id, decision, &state).await
        }
        ScreenAction::GrantMakeupAttempt => {
            review::grant_makeup_attempt(screen, message_id, &state).await
        }
    }
}

async fn handle_flow_action(
    screen: Screen<'_>,
    telegram_user_id: TelegramUserId,
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
        FlowAction::ChangeReviewDecision(submission_id) => {
            review::change_decision(screen, telegram_user_id, submission_id, &state).await?;
        }
        FlowAction::Grading(choice) => {
            teacher::handle_grading(screen, message_id, choice, state).await?;
        }
        FlowAction::LatePolicy(choice) => {
            teacher::handle_late_policy(screen, message_id, choice, state).await?;
        }
        FlowAction::Penalty(choice) => {
            teacher::handle_penalty(screen, message_id, choice, state).await?
        }
        FlowAction::Periods(choice) => {
            teacher::handle_periods(screen, message_id, choice, state).await?
        }
        FlowAction::PenaltyPeriod => teacher::handle_period(screen, message_id, state).await?,
        FlowAction::Terms(action) => {
            teacher::handle_terms(screen, message_id, action, state).await?
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
        FlowAction::MakeupAnswer(choice) => {
            if state.active_makeup().is_some() {
                teacher::handle_makeup_answer(screen, message_id, choice, state).await?;
            } else {
                teacher::handle_makeup(screen, message_id, choice, state).await?;
            }
        }
        FlowAction::MakeupAttempts(choice) => {
            teacher::handle_makeup_attempts(screen, message_id, choice, state).await?;
        }
        FlowAction::MakeupForm(action) => {
            teacher::handle_makeup_form(screen, message_id, action, state).await?;
        }
    }

    Ok(())
}

async fn ensure_private_message(bot: &Bot, message: &Message) -> HandlerResult<bool> {
    if message.chat.is_private() {
        return Ok(true);
    }

    bot.send_message(message.chat.id, texts::PRIVATE_CHATS_ONLY)
        .await?;
    Ok(false)
}

fn message_sender_id(message: &Message) -> HandlerResult<TelegramUserId> {
    sender_id(
        message
            .from
            .as_ref()
            .context("private Telegram message has no sender")?,
    )
}

fn sender_id(user: &User) -> HandlerResult<TelegramUserId> {
    let value = i64::try_from(user.id.0).context("Telegram user id does not fit into i64")?;
    TelegramUserId::new(value).map_err(|error| anyhow!(error).into())
}

fn button_hint(state: &BotState) -> Option<&'static str> {
    match state {
        BotState::Flow(FlowState::AwaitingReviewDecision { .. }) => {
            Some(texts::CHOOSE_REVIEW_DECISION)
        }
        BotState::Screen(ScreenState::ManagingAssignment { .. }) => {
            Some(texts::CHOOSE_ASSIGNMENT_ACTION)
        }
        BotState::Screen(
            ScreenState::GroupInvite { .. }
            | ScreenState::SandboxSubmissions { .. }
            | ScreenState::MakeupReviewed { .. }
            | ScreenState::StudentGrades { .. },
        )
        | BotState::Flow(FlowState::ConfirmingInviteRotation { .. }) => {
            Some(texts::CHOOSE_ACTION_ABOVE)
        }
        BotState::Screen(ScreenState::ManagingAssignmentFiles { .. }) => {
            Some(texts::CHOOSE_FILE_ACTION)
        }
        BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. }) => {
            Some(texts::CONFIRM_FILE_DELETION)
        }
        BotState::Flow(FlowState::ConfirmingTermsExit { .. }) => Some(texts::CHOOSE_BUTTON),
        BotState::Flow(FlowState::ChoosingStudent { .. }) => Some(texts::CHOOSE_STUDENT),
        BotState::Idle
        | BotState::Screen(ScreenState::Guest)
        | BotState::Flow(
            FlowState::AwaitingRegistrationName { .. }
            | FlowState::AwaitingSubmission { .. }
            | FlowState::AwaitingReviewScore { .. }
            | FlowState::AwaitingReviewComment { .. }
            | FlowState::AwaitingBulkReviewComment { .. }
            | FlowState::AwaitingAssignmentFile { .. }
            | FlowState::AwaitingAssignmentTitle { .. }
            | FlowState::AwaitingAssignmentBody { .. }
            | FlowState::AwaitingAssignmentMaxAttempts { .. }
            | FlowState::ChoosingGrading { .. }
            | FlowState::AwaitingGradingText { .. }
            | FlowState::ConfiguringTerms(_)
            | FlowState::ConfiguringMakeup(_)
            | FlowState::AwaitingAttemptAmount { .. },
        ) => None,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use chrono_tz::Tz;

    use super::input::parse_positive_attempts;
    use super::*;
    use crate::bot::BotTimezone;
    use crate::bot::callbacks::{EXTRA_ATTEMPTS_PREFIX, MAX_ATTEMPTS_PREFIX};
    use crate::bot::views::{
        assignment_actions_keyboard, attempts_keyboard, review_keyboard, student_menu, teacher_menu,
    };
    use crate::domain::{SubmissionId, TeachingCourseId, UserId};

    fn timezone() -> BotTimezone {
        BotTimezone::new(Tz::Asia__Vladivostok)
    }

    #[test]
    fn command_names_match_the_declared_bot_commands() {
        for declared in Command::bot_commands() {
            let parsed = Command::parse(&declared.command, "test_bot").unwrap();

            assert_eq!(declared.command.trim_start_matches('/'), parsed.name());
        }
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
            let action = menu_action_for(&Command::parse(input, "test_bot").unwrap());
            assert_eq!(action, Some(expected), "{input}");
        }
    }

    #[test]
    fn legacy_menu_buttons_commands_and_first_pages_share_a_route() {
        for (command, legacy, target) in [
            (
                "/tasks",
                callbacks::MENU_ASSIGNMENTS,
                PageTarget::Assignments,
            ),
            ("/archive", callbacks::MENU_ARCHIVE, PageTarget::Archive),
            ("/queue", callbacks::MENU_QUEUE, PageTarget::Queue),
            ("/courses", callbacks::MENU_COURSES, PageTarget::Courses),
        ] {
            let menu = menu_action_for(&Command::parse(command, "test_bot").unwrap()).unwrap();
            let expected = menu_target(menu).map(CallbackAction::Replayable);
            assert_eq!(callbacks::parse(legacy), expected, "{legacy}");
            assert_eq!(callbacks::parse(&target.callback(0)), expected, "{command}");
            assert_eq!(menu_target(menu).unwrap().callback(), legacy);
        }
    }

    #[test]
    fn every_button_the_bot_sends_is_routable() {
        use crate::app::{MakeupQuestion, UndecidedAssignment};
        use crate::bot::assignment_form::{AssignmentTerms, TermsForm, TermsStep};
        use crate::bot::callbacks::MakeupFormAction;
        use crate::bot::makeup_form::{MakeupForm, MakeupStep};
        use crate::bot::views::{
            assignment_files_keyboard, assignment_students_keyboard, attempts_keyboard,
            calendar_keyboard, confirm_file_deletion_keyboard, confirm_invite_rotation_keyboard,
            contact_keyboard, course_assignments_keyboard, deadline_options_keyboard,
            gradebook_students_keyboard, grading_keyboard, group_invite_keyboard,
            late_policy_keyboard, makeup_answer_keyboard, makeup_attempts_keyboard,
            makeup_followup_keyboard, makeup_form_view, penalty_keyboard, review_decision_keyboard,
            review_hub_keyboard, review_score_keyboard, sandbox_keyboard, student_grades_keyboard,
            terms_exit_keyboard, terms_form_view, terms_keyboard,
        };
        use crate::domain::{
            CourseMakeup, DeductedFrom, FilePresentation, Grading, StoredFileId,
            StoredFileMetadata, StoredFileOwner, StoredFileParts,
        };
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let assignment = assignment(&course, &teacher).scored(5).published().build();
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
        let student_id = UserId::new();
        let contact = InformationalAction::StudentContact {
            course_id: course.id(),
            student_id,
        };

        let terms = AssignmentTerms {
            deducted_from: Some(DeductedFrom::CourseTotal),
            penalty_per_period: Some(1),
            max_periods: Some(4),
            ..AssignmentTerms::editing(
                &assignment,
                0,
                MakeupQuestion {
                    enabled: true,
                    hint: Some(3),
                },
            )
        };
        let sandbox_submission = crate::app::SandboxSubmission {
            submission_id: SubmissionId::new(),
            kind: crate::domain::SubmissionKind::Makeup,
            assignment_title: "Trial".to_owned(),
            student_name: "Student".to_owned(),
            submitted_at: Utc::now(),
        };

        let mut keyboards = vec![
            student_menu(),
            teacher_menu(Some(1)),
            review_keyboard(
                SubmissionId::new(),
                course.id(),
                student_id,
                Some(texts::BUTTON_ACCEPT.to_owned()),
            ),
            review_keyboard(SubmissionId::new(), course.id(), student_id, None),
            review_decision_keyboard(SubmissionId::new()),
            review_hub_keyboard(12, 5),
            makeup_followup_keyboard(true, true),
            student_grades_keyboard(course.id(), student_id, true),
            contact_keyboard(
                contact,
                Some("https://t.me/student_name".parse().unwrap()),
                texts::BUTTON_REFRESH_CONTACT,
            ),
            contact_keyboard(
                contact,
                Some("tg://user?id=12345".parse().unwrap()),
                texts::BUTTON_REFRESH_CONTACT,
            ),
            contact_keyboard(contact, None, texts::BUTTON_REFRESH_CONTACT),
            contact_keyboard(contact, None, texts::BUTTON_RETRY_CONTACT),
            course_assignments_keyboard(
                course.id(),
                std::slice::from_ref(&assignment),
                None,
                &timezone,
            ),
            course_assignments_keyboard(
                course.id(),
                &[],
                Some(
                    CourseMakeup::open(
                        crate::test_support::t0() + chrono::Duration::days(30),
                        3,
                        crate::test_support::t0(),
                    )
                    .unwrap(),
                ),
                &timezone,
            ),
            makeup_attempts_keyboard(),
            makeup_answer_keyboard(Grading::scored(10).unwrap(), Some(6)),
            makeup_answer_keyboard(Grading::scored(20).unwrap(), Some(12)),
            makeup_answer_keyboard(Grading::PassFail, None),
            assignment_actions_keyboard(&assignment),
            attempts_keyboard(MAX_ATTEMPTS_PREFIX),
            attempts_keyboard(EXTRA_ATTEMPTS_PREFIX),
            grading_keyboard(),
            late_policy_keyboard(),
            penalty_keyboard(7),
            terms_keyboard(&terms, &terms.complete().unwrap(), &timezone),
            terms_exit_keyboard(),
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
            sandbox_keyboard(std::slice::from_ref(&sandbox_submission), 1, Some(2)),
        ];

        let opened = CourseMakeup::open(
            crate::test_support::t0() + chrono::Duration::days(30),
            3,
            crate::test_support::t0(),
        )
        .unwrap();
        let undecided = vec![UndecidedAssignment {
            assignment_id: assignment.id(),
            title: "Lab".to_owned(),
            grading: assignment.grading(),
        }];
        let mut forms = vec![MakeupForm::new(
            course.id(),
            None,
            undecided.clone(),
            crate::test_support::t0(),
        )];
        for step in [MakeupStep::Attempts, MakeupStep::Scale(0)] {
            let mut form = MakeupForm::new(
                course.id(),
                None,
                undecided.clone(),
                crate::test_support::t0(),
            );
            form.go(step);
            forms.push(form);
        }
        let mut summary = MakeupForm::new(
            course.id(),
            Some(opened),
            Vec::new(),
            crate::test_support::t0(),
        );
        forms.push(summary.clone());
        assert!(summary.edit(MakeupFormAction::Disable));
        forms.push(summary);
        let mut calendar =
            MakeupForm::new(course.id(), None, Vec::new(), crate::test_support::t0());
        calendar.mode = crate::bot::assignment_form::InputMode::Calendar(today);
        forms.push(calendar);
        for form in forms {
            let (_, keyboard) =
                makeup_form_view(&form, &timezone, crate::test_support::t0()).unwrap();
            keyboards.push(keyboard);
        }

        for step in [
            TermsStep::Makeup,
            TermsStep::Policy,
            TermsStep::Deadline,
            TermsStep::Penalty,
            TermsStep::Period,
            TermsStep::Periods,
            TermsStep::DurationDate,
            TermsStep::DurationConfirm {
                selected: assignment.deadline_at() + chrono::Duration::days(10),
                periods: 2,
            },
            TermsStep::Summary,
        ] {
            let (_, keyboard) = terms_form_view(
                &TermsForm::new(terms.clone(), step),
                &timezone,
                crate::test_support::t0(),
            )
            .unwrap();
            keyboards.push(keyboard);
        }

        for keyboard in keyboards {
            for button in keyboard.inline_keyboard.into_iter().flatten() {
                use teloxide::types::InlineKeyboardButtonKind;
                match button.kind {
                    InlineKeyboardButtonKind::CallbackData(data) => {
                        assert!(
                            callbacks::parse(&data).is_some(),
                            "button {:?} sends {data}, which the callback parser rejects",
                            button.text,
                        );
                        assert!(data.len() <= 64, "{data} exceeds the Telegram limit");
                    }
                    InlineKeyboardButtonKind::Url(url) => {
                        assert!(
                            matches!(
                                url.as_str(),
                                "https://t.me/student_name" | "tg://user?id=12345"
                            ),
                            "unexpected Telegram URL: {url}"
                        );
                    }
                    kind => panic!("unverified button kind: {kind:?}"),
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
        let keyboard = review_keyboard(
            submission_id,
            TeachingCourseId::new(),
            UserId::new(),
            Some(texts::BUTTON_ACCEPT.to_owned()),
        );
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
