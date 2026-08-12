use chrono::Utc;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::app::{AssignmentStudentSummary, TeachingCourseSummary};
use crate::domain::{
    Assignment, AssignmentStatus, StoredFileMetadata, SubmissionId, SubmissionStatus,
    TeachingCourseId,
};

use super::HandlerResult;

const MESSAGE_LIMIT: usize = 4096;

pub(super) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "Мои задания",
        "menu:assignments",
    )]])
}

pub(super) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || "Проверить следующую".to_owned(),
        |pending| format!("Проверить следующую ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(review_label, "review:next")],
        vec![InlineKeyboardButton::callback("Мои курсы", "menu:courses")],
    ])
}

pub(super) fn group_keyboard(group_codes: &[String]) -> InlineKeyboardMarkup {
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

pub(super) fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
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

pub(super) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        courses
            .iter()
            .map(|course| {
                vec![InlineKeyboardButton::callback(
                    button_label(&format!(
                        "{} · {}",
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
                    "{} · {}",
                    assignment.title().as_str(),
                    assignment_status(assignment.status())
                )),
                format!("manage:{}", assignment.id().as_uuid()),
            )]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "Создать черновик",
        format!("create:{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "К списку курсов",
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
                "Опубликовать",
                format!("publish:{assignment_id}"),
            )]);
            rows.push(vec![InlineKeyboardButton::callback(
                "Архивировать",
                format!("archive:{assignment_id}"),
            )]);
        }
        AssignmentStatus::Published => rows.push(vec![InlineKeyboardButton::callback(
            "Архивировать",
            format!("archive:{assignment_id}"),
        )]),
        AssignmentStatus::Archived => rows.push(vec![InlineKeyboardButton::callback(
            "Опубликовать снова",
            format!("publish:{assignment_id}"),
        )]),
    }

    if assignment.status() != AssignmentStatus::Archived {
        rows.push(vec![InlineKeyboardButton::callback(
            "Изменить дедлайн",
            format!("deadline:{assignment_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "Вложения",
        format!("files:{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "Выдать дополнительные попытки",
        format!("attempts:{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "К заданиям курса",
        format!("course:{}", assignment.teaching_course_id().as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
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
                button_label(file.filename().as_str()),
                format!("file:open:{index}"),
            ),
            InlineKeyboardButton::callback("Удалить", format!("file:delete:{index}")),
        ]);
    }
    if can_add {
        rows.push(vec![InlineKeyboardButton::callback(
            "Добавить вложение",
            "file:add",
        )]);
    }
    if creation_flow {
        rows.push(vec![InlineKeyboardButton::callback(
            "Завершить создание",
            "file:finish",
        )]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(
            "К заданию",
            format!("manage:{}", assignment.id().as_uuid()),
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn confirm_file_deletion_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Удалить", "file:confirm-delete"),
        InlineKeyboardButton::callback("Отмена", "file:cancel-delete"),
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
                button_label(student.student.display_name().as_str()),
                format!("student:{}", student.student.id().as_uuid()),
            )]
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "К заданию",
        format!("manage:{}", assignment_id.as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn format_deadline(deadline: chrono::DateTime<Utc>) -> String {
    deadline.format("%d.%m.%Y %H:%M UTC").to_string()
}

pub(super) fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "на проверке",
        SubmissionStatus::Accepted => "принята",
        SubmissionStatus::Rejected => "отклонена",
    }
}

pub(super) fn assignment_status(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "черновик",
        AssignmentStatus::Published => "опубликовано",
        AssignmentStatus::Archived => "архив",
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

pub(super) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, "Эта кнопка устарела. Откройте /menu ещё раз.")
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
