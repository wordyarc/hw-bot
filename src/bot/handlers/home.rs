use teloxide::payloads::{SendMessageSetters, SetMyCommandsSetters};
use teloxide::prelude::{Bot, Requester};
use teloxide::types::{BotCommand, BotCommandScope, ChatId, Recipient};
use teloxide::utils::command::BotCommands;

use super::Command;
use crate::app::{Actor, AppResult, ErrorCode};
use crate::bot::context::Screen;
use crate::bot::errors::send_app_error;
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{
    BUTTON_ARCHIVE, BUTTON_ASSIGNMENTS, BUTTON_COURSES, BUTTON_QUEUE, BUTTON_REVIEW, main_keyboard,
    role_name, send_message_chunks, student_menu, teacher_menu,
};
use crate::bot::{BotServices, HandlerResult};
use crate::domain::{Role, TelegramUserId};

const STUDENT_COMMANDS: [&str; 7] = [
    "menu", "tasks", "queue", "archive", "profile", "cancel", "help",
];
const TEACHER_COMMANDS: [&str; 6] = ["menu", "review", "courses", "profile", "cancel", "help"];
const GUEST_COMMANDS: [&str; 3] = ["start", "menu", "help"];

pub(in crate::bot) fn default_commands() -> Vec<BotCommand> {
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

pub(super) async fn send_help(
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
                command.description
            )
        })
        .collect()
}

fn optional_stat<T>(result: AppResult<T>, statistic: &'static str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(statistic, error = ?error, "optional profile statistic is unavailable");
            None
        }
    }
}

pub(super) async fn show_home(
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
                    let pending = optional_stat(
                        services
                            .submissions
                            .pending_queue(Actor::new(registered.user.id()))
                            .await,
                        "student_pending_queue",
                    )
                    .map(|places| places.len());
                    match pending {
                        Some(pending) if pending > 0 => format!(
                            "👋 Здравствуйте, {name}!\n⏳ На проверке ваших работ: {pending}\nВыберите пункт меню ниже 👇"
                        ),
                        _ => format!("👋 Здравствуйте, {name}!\nВыберите пункт меню ниже 👇"),
                    }
                }
                Role::Teacher | Role::Admin => {
                    let pending = match services
                        .reviews
                        .queue_size(Actor::new(registered.user.id()))
                        .await
                    {
                        Ok(pending) => pending,
                        Err(error) => return send_app_error(bot, chat_id, error).await,
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
            bot.send_message(chat_id, texts::GUEST_NEEDS_INVITE).await?;
        }
        Err(error) => send_app_error(bot, chat_id, error).await?,
    }
    Ok(())
}

pub(super) async fn show_profile(screen: Screen<'_>) -> HandlerResult {
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
        Err(error) => return send_app_error(bot, chat_id, error).await,
    };
    let role = registered.user.role();
    let mut text = format!(
        "👤 Профиль\n\nИмя: {}\nРоль: {}\n",
        registered.user.display_name().as_str(),
        role_name(role)
    );
    if let Some(group) = &registered.student_group {
        text.push_str(&format!("Группа: {}\n", group.code().as_str()));
    }
    text.push_str(&format!("🕒 Часовой пояс: {}\n", timezone.suffix()));
    match role {
        Role::Student { .. } => {
            if let Some(assignments) = optional_stat(
                services.assignments.list_for_student(actor).await,
                "student_assignments",
            ) {
                text.push_str(&format!("\n📚 Активных заданий: {}\n", assignments.len()));
            }
            if let Some(places) = optional_stat(
                services.submissions.pending_queue(actor).await,
                "student_pending_queue",
            ) {
                text.push_str(&format!("⏳ Работ на проверке: {}\n", places.len()));
            }
        }
        Role::Teacher | Role::Admin => {
            if let Some(courses) = optional_stat(
                services.courses.list_for_teacher(actor).await,
                "teacher_courses",
            ) {
                text.push_str(&format!("\n📘 Курсов: {}\n", courses.len()));
            }
            if let Some(pending) = optional_stat(
                services.reviews.queue_size(actor).await,
                "teacher_pending_queue",
            ) {
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
}
