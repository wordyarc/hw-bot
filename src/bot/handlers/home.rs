use teloxide::payloads::{SendMessageSetters, SetMyCommandsSetters};
use teloxide::types::{BotCommand, BotCommandScope, ChatId, KeyboardRemove, Recipient};
use teloxide::utils::command::BotCommands;

use super::Command;
use crate::app::{Actor, ErrorCode};
use crate::bot::Bot;
use crate::bot::HandlerResult;
use crate::bot::context::{Chat, Screen};
use crate::bot::errors::{log_request_error, optional_stat};
use crate::bot::state::{BotState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{main_keyboard, main_menu, send_message_chunks};
use crate::domain::{Role, TelegramUserId, User};

pub(crate) fn default_commands() -> Vec<BotCommand> {
    scoped_commands(None)
}

fn scoped_commands(role: Option<Role>) -> Vec<BotCommand> {
    Command::bot_commands()
        .into_iter()
        .filter_map(|mut command| {
            let parsed = Command::parse(&command.command, "")
                .expect("declared bot command must be parseable without arguments");
            if !parsed.visible_to(role) {
                return None;
            }
            command.command = command.command.trim_start_matches('/').to_owned();
            if role.is_none() {
                command.description = match parsed {
                    Command::Start(_) => texts::GUEST_START_DESCRIPTION.to_owned(),
                    Command::Help => texts::GUEST_HELP_DESCRIPTION.to_owned(),
                    _ => command.description,
                };
            }
            Some(command)
        })
        .collect()
}

async fn sync_commands(bot: &Bot, chat_id: ChatId, role: Option<Role>) {
    if let Err(error) = bot
        .set_my_commands(scoped_commands(role))
        .scope(BotCommandScope::Chat {
            chat_id: Recipient::Id(chat_id),
        })
        .await
    {
        log_request_error("set_role_commands", &error);
    }
}

pub(crate) async fn open_guest(chat: Chat<'_>) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    show_guest(bot, chat_id).await?;
    chat.dialogue()
        .update(BotState::Screen(ScreenState::Guest))
        .await?;
    Ok(())
}

async fn show_guest(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    sync_commands(bot, chat_id, None).await;
    send_guest_hint(bot, chat_id).await
}

pub(super) async fn send_guest_hint(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, texts::GUEST_NEEDS_INVITE)
        .reply_markup(KeyboardRemove::new())
        .await?;
    Ok(())
}

pub(super) async fn send_help(chat: Chat<'_>, telegram_user_id: TelegramUserId) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let services = chat.services();
    let timezone = chat.timezone();
    let role = match services
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(registered) => Some(registered.user.role()),
        Err(error) if error.code() == ErrorCode::NotFound => {
            return show_guest(bot, chat_id).await;
        }
        Err(error) => return Err(error.into()),
    };
    let text = format!(
        "{}\n\n{}\n\n{}",
        help_for(role),
        texts::timezone_note(&timezone.label_at(chat.now())),
        help_commands(role),
    );
    send_message_chunks(bot, chat_id, text, None).await
}

fn help_for(role: Option<Role>) -> String {
    match role {
        Some(Role::Student { .. }) => texts::student_help(),
        Some(Role::Teacher | Role::Admin) => texts::teacher_help(),
        None => texts::GUEST_NEEDS_INVITE.to_owned(),
    }
}

fn help_commands(role: Option<Role>) -> String {
    scoped_commands(role)
        .into_iter()
        .map(|command| {
            format!(
                "/{} — {}\n",
                command.command.trim_start_matches('/'),
                command.description
            )
        })
        .collect()
}

pub(super) async fn show_home(chat: Chat<'_>, telegram_user_id: TelegramUserId) -> HandlerResult {
    match chat
        .services()
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await
    {
        Ok(registered) => send_home(chat, &registered.user).await,
        Err(error) if error.code() == ErrorCode::NotFound => open_guest(chat).await,
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn send_home(chat: Chat<'_>, user: &User) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let services = chat.services();
    let role = user.role();
    sync_commands(bot, chat_id, Some(role)).await;
    let name = user.display_name().as_str();
    let actor = Actor::new(user.id());
    let text = match role {
        Role::Student { .. } => {
            let pending = optional_stat(
                services.users.statistics(actor).await,
                "student_pending_queue",
            )
            .map(|stats| stats.pending);
            match pending {
                Some(pending) if pending > 0 => format!(
                    "{}\n\n{}",
                    texts::home_greeting(name),
                    texts::student_pending_line(pending)
                ),
                _ => texts::home_greeting(name),
            }
        }
        Role::Teacher | Role::Admin => {
            let mut lines = Vec::new();
            let pending = optional_stat(
                services.reviews.queue_size(actor).await,
                "teacher_pending_queue",
            );
            if let Some(pending) = pending {
                lines.push(texts::review_queue_line(pending));
            }
            let makeups =
                optional_stat(services.reviews.makeup_size(actor).await, "teacher_makeups");
            if let Some(makeups) = makeups.filter(|&count| count > 0) {
                lines.push(texts::makeup_queue_line(makeups));
            }
            let sandbox = optional_stat(
                services.reviews.sandbox_size(actor, None).await,
                "teacher_sandbox",
            );
            if let Some(sandbox) = sandbox.filter(|&count| count > 0) {
                lines.push(texts::home_sandbox_line(sandbox));
            }
            let greeting = texts::home_greeting(name);
            if lines.is_empty() {
                greeting
            } else {
                format!("{greeting}\n\n{}", lines.join("\n"))
            }
        }
    };
    bot.send_message(chat_id, text)
        .reply_markup(main_keyboard(role))
        .await?;
    Ok(())
}

pub(super) async fn show_profile(screen: Screen<'_>) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let services = screen.services();
    let timezone = screen.timezone();
    let registered = services.users.find_by_actor(actor).await?;
    let role = registered.user.role();
    let mut lines = vec![texts::profile_header(
        registered.user.display_name().as_str(),
        texts::role_name(role),
    )];
    if let Some(group) = &registered.student_group {
        lines.push(texts::profile_group(group.code().as_str()));
    }
    lines.push(format!(
        "{} {}",
        texts::TIMEZONE_LABEL,
        timezone.label_at(screen.now())
    ));
    match role {
        Role::Student { .. } => {
            if let Some(stats) =
                optional_stat(services.users.statistics(actor).await, "student_statistics")
            {
                lines.push(String::new());
                lines.push(texts::profile_active_assignments(stats.assignments));
                lines.push(texts::profile_pending(stats.pending));
            }
        }
        Role::Teacher | Role::Admin => {
            if let Some(stats) =
                optional_stat(services.users.statistics(actor).await, "teacher_statistics")
            {
                lines.push(String::new());
                lines.push(texts::profile_courses(stats.courses));
            }
            if let Some(pending) = optional_stat(
                services.reviews.queue_size(actor).await,
                "teacher_pending_queue",
            ) {
                lines.push(texts::profile_review_queue(pending));
            }
        }
    }
    let keyboard = main_menu(role, None);
    send_message_chunks(bot, chat_id, lines.join("\n"), Some(keyboard)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::StudentGroupId;

    #[test]
    fn command_hints_are_scoped_to_the_role() {
        let student = names(scoped_commands(Some(Role::Student {
            student_group_id: StudentGroupId::new(),
        })));
        let teacher = names(scoped_commands(Some(Role::Teacher)));

        assert_eq!(
            student,
            [
                "menu", "tasks", "queue", "archive", "profile", "cancel", "help"
            ]
        );
        assert_eq!(
            teacher,
            ["menu", "review", "courses", "profile", "cancel", "help"]
        );
        assert_eq!(names(scoped_commands(Some(Role::Admin))), teacher);
        assert_eq!(names(default_commands()), ["start", "help"]);
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

        assert!(student.contains(texts::BUTTON_SUBMIT_MAKEUP), "{student}");
        assert!(teacher.contains("Досдача включается"), "{teacher}");
        assert!(teacher.contains(texts::BUTTON_REVIEW), "{teacher}");
        assert!(!student.contains("Досдача включается"), "{student}");
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
}
