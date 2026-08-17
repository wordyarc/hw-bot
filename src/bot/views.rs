use chrono::{Datelike, NaiveDate, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, KeyboardRemove,
};

use crate::app::{AssignmentStudentSummary, TeachingCourseSummary};
use crate::domain::{
    Assignment, AssignmentStatus, Role, StoredFileMetadata, SubmissionId, SubmissionStatus,
    TeachingCourseId,
};

use super::HandlerResult;
use super::timezone::BotTimezone;

const MESSAGE_LIMIT: usize = 4096;

pub(super) const BUTTON_ASSIGNMENTS: &str = "📚 Мои задания";
pub(super) const BUTTON_ARCHIVE: &str = "🗄 Архив";
pub(super) const BUTTON_QUEUE: &str = "⏳ Очередь проверки";
pub(super) const BUTTON_REVIEW: &str = "✅ Проверить работы";
pub(super) const BUTTON_COURSES: &str = "📘 Мои курсы";
pub(super) const BUTTON_PROFILE: &str = "👤 Профиль";
pub(super) const BUTTON_HELP: &str = "❓ Помощь";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuAction {
    Assignments,
    Archive,
    Queue,
    Review,
    Courses,
    Profile,
    Help,
}

pub(super) fn menu_action(text: &str) -> Option<MenuAction> {
    match text.trim() {
        BUTTON_ASSIGNMENTS => Some(MenuAction::Assignments),
        BUTTON_ARCHIVE => Some(MenuAction::Archive),
        BUTTON_QUEUE => Some(MenuAction::Queue),
        BUTTON_REVIEW => Some(MenuAction::Review),
        BUTTON_COURSES => Some(MenuAction::Courses),
        BUTTON_PROFILE => Some(MenuAction::Profile),
        BUTTON_HELP => Some(MenuAction::Help),
        _ => None,
    }
}

pub(super) fn main_keyboard(role: Role) -> KeyboardMarkup {
    match role {
        Role::Student { .. } => student_keyboard(),
        Role::Teacher | Role::Admin => teacher_keyboard(),
    }
}

pub(super) fn student_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![
            KeyboardButton::new(BUTTON_ASSIGNMENTS),
            KeyboardButton::new(BUTTON_QUEUE),
        ],
        vec![
            KeyboardButton::new(BUTTON_ARCHIVE),
            KeyboardButton::new(BUTTON_PROFILE),
        ],
        vec![KeyboardButton::new(BUTTON_HELP)],
    ])
}

pub(super) fn teacher_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![KeyboardButton::new(BUTTON_REVIEW)],
        vec![
            KeyboardButton::new(BUTTON_COURSES),
            KeyboardButton::new(BUTTON_PROFILE),
        ],
        vec![KeyboardButton::new(BUTTON_HELP)],
    ])
}

fn bottom_menu(rows: Vec<Vec<KeyboardButton>>) -> KeyboardMarkup {
    KeyboardMarkup::new(rows)
        .resize_keyboard()
        .persistent()
        .input_field_placeholder("Выберите пункт меню")
}

pub(super) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            BUTTON_ASSIGNMENTS,
            "menu:assignments",
        )],
        vec![
            InlineKeyboardButton::callback(BUTTON_QUEUE, "menu:queue"),
            InlineKeyboardButton::callback(BUTTON_ARCHIVE, "menu:archive"),
        ],
        vec![InlineKeyboardButton::callback(
            BUTTON_PROFILE,
            "menu:profile",
        )],
    ])
}

pub(super) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || BUTTON_REVIEW.to_owned(),
        |pending| format!("✅ Проверить работы ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(review_label, "review:next")],
        vec![InlineKeyboardButton::callback(
            BUTTON_COURSES,
            "menu:courses",
        )],
        vec![InlineKeyboardButton::callback(
            BUTTON_PROFILE,
            "menu:profile",
        )],
    ])
}

pub(super) fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "✅ Принять",
            format!("review:accept:{}", submission_id.as_uuid()),
        ),
        InlineKeyboardButton::callback(
            "❌ Отклонить",
            format!("review:reject:{}", submission_id.as_uuid()),
        ),
    ]])
}

pub(super) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        courses
            .iter()
            .map(|course| {
                vec![InlineKeyboardButton::callback(
                    button_label(&format!(
                        "📘 {} · {}",
                        course.subject.name().as_str(),
                        course.group.code().as_str()
                    )),
                    format!("course:{}", course.course.id().as_uuid()),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn course_assignments_keyboard(
    course_id: TeachingCourseId,
    assignments: &[Assignment],
) -> InlineKeyboardMarkup {
    let mut rows: Vec<_> = assignments
        .iter()
        .map(|assignment| {
            vec![InlineKeyboardButton::callback(
                button_label(&format!(
                    "{} {}",
                    assignment_status_icon(assignment.status()),
                    assignment.title().as_str(),
                )),
                format!("manage:{}", assignment.id().as_uuid()),
            )]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "➕ Создать задание",
        format!("create:{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🔗 Ссылка для группы",
        format!("invite:{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К списку курсов",
        "menu:courses",
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn assignment_actions_keyboard(assignment: &Assignment) -> InlineKeyboardMarkup {
    let assignment_id = assignment.id().as_uuid();
    let mut rows = Vec::new();

    match assignment.status() {
        AssignmentStatus::Draft => {
            rows.push(vec![InlineKeyboardButton::callback(
                "🚀 Опубликовать",
                format!("publish:{assignment_id}"),
            )]);
            rows.push(vec![InlineKeyboardButton::callback(
                "📦 Архивировать",
                format!("archive:{assignment_id}"),
            )]);
        }
        AssignmentStatus::Published => rows.push(vec![InlineKeyboardButton::callback(
            "📦 Архивировать",
            format!("archive:{assignment_id}"),
        )]),
        AssignmentStatus::Archived => rows.push(vec![InlineKeyboardButton::callback(
            "🚀 Опубликовать снова",
            format!("publish:{assignment_id}"),
        )]),
    }

    if assignment.status() != AssignmentStatus::Archived {
        rows.push(vec![InlineKeyboardButton::callback(
            "⏰ Изменить дедлайн",
            format!("deadline:{assignment_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "📎 Вложения",
        format!("files:{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🎁 Выдать попытки",
        format!("attempts:{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К заданиям курса",
        format!("course:{}", assignment.teaching_course_id().as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn deadline_options_keyboard(timezone: &BotTimezone) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    let mut preset = |label: &str, date: Option<NaiveDate>, callback: &str| {
        if let Some(date) = date {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("📅 {label} · {}", timezone.format_date(date)),
                callback.to_owned(),
            )]);
        }
    };

    preset("Через неделю", timezone.in_days(7), "dl:days:7");
    preset("Через 2 недели", timezone.in_days(14), "dl:days:14");
    preset("Через месяц", timezone.in_months(1), "dl:months:1");
    rows.push(vec![InlineKeyboardButton::callback(
        "🗓 Выбрать дату",
        "dl:calendar",
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "✍️ Ввести вручную",
        "dl:manual",
    )]);

    InlineKeyboardMarkup::new(rows)
}

const MONTHS: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

pub(super) fn calendar_keyboard(month: NaiveDate, today: NaiveDate) -> InlineKeyboardMarkup {
    let blank = || InlineKeyboardButton::callback("·", "cal:noop");
    let first = month.with_day(1).unwrap_or(month);
    let previous = first.checked_sub_months(chrono::Months::new(1));
    let next = first.checked_add_months(chrono::Months::new(1));

    let back = match previous {
        Some(previous) if previous >= today.with_day(1).unwrap_or(today) => {
            InlineKeyboardButton::callback("◀️", format!("cal:{}", month_key(previous)))
        }
        _ => blank(),
    };
    let forward = next.map_or_else(blank, |next| {
        InlineKeyboardButton::callback("▶️", format!("cal:{}", month_key(next)))
    });

    let mut rows = vec![
        vec![
            back,
            InlineKeyboardButton::callback(
                format!("{} {}", MONTHS[(first.month() - 1) as usize], first.year()),
                "cal:noop",
            ),
            forward,
        ],
        ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"]
            .into_iter()
            .map(|day| InlineKeyboardButton::callback(day, "cal:noop"))
            .collect(),
    ];

    let mut week = vec![blank(); first.weekday().num_days_from_monday() as usize];
    let mut day = first;
    while day.month() == first.month() {
        week.push(if day < today {
            blank()
        } else if day == today {
            InlineKeyboardButton::callback(
                format!("»{}", day.day()),
                format!("cal:pick:{}", day.format("%Y-%m-%d")),
            )
        } else {
            InlineKeyboardButton::callback(
                day.day().to_string(),
                format!("cal:pick:{}", day.format("%Y-%m-%d")),
            )
        });
        if week.len() == 7 {
            rows.push(std::mem::take(&mut week));
        }
        let Some(tomorrow) = day.succ_opt() else {
            break;
        };
        day = tomorrow;
    }
    if !week.is_empty() {
        week.resize(7, blank());
        rows.push(week);
    }

    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Назад",
        "dl:options",
    )]);

    InlineKeyboardMarkup::new(rows)
}

fn month_key(date: NaiveDate) -> String {
    date.format("%Y-%m").to_string()
}

pub(super) fn attempts_keyboard(prefix: &str) -> InlineKeyboardMarkup {
    let button = |amount: u16| {
        InlineKeyboardButton::callback(amount.to_string(), format!("{prefix}{amount}"))
    };

    InlineKeyboardMarkup::new(vec![
        [1, 2, 3].into_iter().map(button).collect(),
        [5, 10].into_iter().map(button).collect(),
        vec![InlineKeyboardButton::callback(
            "✍️ Другое число",
            format!("{prefix}manual"),
        )],
    ])
}

pub(super) fn assignment_files_keyboard(
    assignment: &Assignment,
    files: &[StoredFileMetadata],
    can_add: bool,
    creation_flow: bool,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for (index, file) in files.iter().enumerate() {
        rows.push(vec![
            InlineKeyboardButton::callback(
                button_label(&format!("📄 {}", file.filename().as_str())),
                format!("file:open:{index}"),
            ),
            InlineKeyboardButton::callback("🗑", format!("file:delete:{index}")),
        ]);
    }
    if can_add {
        rows.push(vec![InlineKeyboardButton::callback(
            "📎 Добавить вложение",
            "file:add",
        )]);
    }
    if creation_flow {
        rows.push(vec![InlineKeyboardButton::callback(
            "✅ Завершить создание",
            "file:finish",
        )]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(
            "⬅️ К заданию",
            format!("manage:{}", assignment.id().as_uuid()),
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn confirm_file_deletion_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🗑 Удалить", "file:confirm-delete"),
        InlineKeyboardButton::callback("↩️ Отмена", "file:cancel-delete"),
    ]])
}

pub(super) fn group_invite_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "♻️ Перевыпустить ссылку",
            "invite:rotate",
        )],
        vec![InlineKeyboardButton::callback(
            "⬅️ К курсу",
            format!("course:{}", course_id.as_uuid()),
        )],
    ])
}

pub(super) fn confirm_invite_rotation_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("♻️ Перевыпустить", "invite:confirm-rotate"),
        InlineKeyboardButton::callback("↩️ Отмена", "invite:cancel-rotate"),
    ]])
}

pub(super) fn format_file_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} Б")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} КиБ", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МиБ", size_bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(super) fn assignment_students_keyboard(
    assignment_id: crate::domain::AssignmentId,
    students: &[AssignmentStudentSummary],
) -> InlineKeyboardMarkup {
    let mut rows = students
        .iter()
        .map(|student| {
            vec![InlineKeyboardButton::callback(
                button_label(&format!("👤 {}", student.student.display_name().as_str())),
                format!("student:{}", student.student.id().as_uuid()),
            )]
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К заданию",
        format!("manage:{}", assignment_id.as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn deadline_left(deadline: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let left = deadline - now;

    if left.num_seconds() <= 0 {
        return "дедлайн прошёл".to_owned();
    }

    let days = left.num_days();
    let hours = left.num_hours() % 24;
    if days > 0 {
        format!("осталось {days} д {hours} ч")
    } else if left.num_hours() > 0 {
        format!("осталось {} ч", left.num_hours())
    } else {
        format!("осталось {} мин", left.num_minutes().max(1))
    }
}

pub(super) fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "⏳ на проверке",
        SubmissionStatus::Accepted => "✅ принята",
        SubmissionStatus::Rejected => "❌ отклонена",
    }
}

pub(super) fn assignment_status(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️ черновик",
        AssignmentStatus::Published => "🟢 опубликовано",
        AssignmentStatus::Archived => "📦 архив",
    }
}

fn assignment_status_icon(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️",
        AssignmentStatus::Published => "🟢",
        AssignmentStatus::Archived => "📦",
    }
}

pub(super) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Student { .. } => "🎓 студент",
        Role::Teacher => "👨‍🏫 преподаватель",
        Role::Admin => "🛠 администратор",
    }
}

pub(super) fn button_label(label: &str) -> String {
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

pub(super) async fn send_message_chunks(
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

pub(super) async fn ask_input(bot: &Bot, chat_id: ChatId, prompt: &str) -> HandlerResult {
    bot.send_message(chat_id, format!("{prompt}\n\n/cancel — отменить"))
        .reply_markup(KeyboardRemove::new())
        .await?;
    Ok(())
}

/// Остаётся только там, где кнопка не может сработать: у мутаций и у кнопок, чей смысл
/// лежит в стейте диалога
pub(super) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(
        chat_id,
        "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu.",
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset, NaiveTime};

    use super::*;

    fn timezone() -> BotTimezone {
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
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
    fn button_labels_are_truncated_only_when_needed() {
        assert_eq!(button_label("Мои задания"), "Мои задания");
        assert_eq!(button_label(&"я".repeat(61)).chars().count(), 63);
        assert!(button_label(&"я".repeat(61)).ends_with("..."));
    }

    #[test]
    fn bottom_menu_buttons_route_back_to_their_screens() {
        assert_eq!(
            menu_action(BUTTON_ASSIGNMENTS),
            Some(MenuAction::Assignments)
        );
        assert_eq!(menu_action(BUTTON_QUEUE), Some(MenuAction::Queue));
        assert_eq!(menu_action(BUTTON_REVIEW), Some(MenuAction::Review));
        assert_eq!(menu_action("ссылка на pull request"), None);
    }

    #[test]
    fn calendar_hides_past_days_and_the_way_back_to_past_months() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let json = serde_json::to_value(calendar_keyboard(today, today)).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks: Vec<_> = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .map(|button| button["callback_data"].as_str().unwrap())
            .collect();

        assert!(!callbacks.contains(&"cal:2026-07"));
        assert!(callbacks.contains(&"cal:2026-09"));
        assert!(!callbacks.contains(&"cal:pick:2026-08-12"));
        assert!(callbacks.contains(&"cal:pick:2026-08-13"));
        assert!(callbacks.contains(&"cal:pick:2026-08-31"));
        assert!(rows.iter().all(|row| row.as_array().unwrap().len() <= 7));
    }

    #[test]
    fn deadline_presets_offer_a_week_two_weeks_and_a_month() {
        let json = serde_json::to_value(deadline_options_keyboard(&timezone())).unwrap();
        let callbacks: Vec<_> = json["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[0]["callback_data"].as_str().unwrap().to_owned())
            .collect();

        assert_eq!(
            callbacks,
            [
                "dl:days:7",
                "dl:days:14",
                "dl:months:1",
                "dl:calendar",
                "dl:manual"
            ]
        );
    }

    #[test]
    fn passed_deadlines_are_not_reported_as_time_left() {
        let past: DateTime<Utc> = Utc::now() - chrono::Duration::hours(1);

        assert_eq!(deadline_left(past, Utc::now()), "дедлайн прошёл");
    }

    #[test]
    fn assignment_action_callbacks_fit_telegram_limit() {
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let draft = assignment(&course, &teacher).build();
        let published = assignment(&course, &teacher).published().build();
        let mut archived = published.clone();
        archived.archive(&course, &teacher).unwrap();

        for assignment in [draft, published, archived] {
            let json = serde_json::to_value(assignment_actions_keyboard(&assignment)).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    assert!(button["callback_data"].as_str().unwrap().len() <= 64);
                }
            }
        }
    }

    #[test]
    fn attachment_callbacks_fit_telegram_limit() {
        use crate::domain::{
            FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner, StoredFileParts,
        };
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let assignment = assignment(&course, &teacher).build();
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

        for keyboard in [
            assignment_files_keyboard(&assignment, &[file], true, true),
            confirm_file_deletion_keyboard(),
        ] {
            let json = serde_json::to_value(keyboard).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    assert!(button["callback_data"].as_str().unwrap().len() <= 64);
                }
            }
        }
    }
}
