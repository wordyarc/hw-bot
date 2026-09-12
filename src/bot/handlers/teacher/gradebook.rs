use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;

use crate::domain::{TeachingCourseId, UserId};

use crate::bot::HandlerResult;
use crate::bot::context::Screen;
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotState, FlowState, StudentChoice};
use crate::bot::views::{gradebook_students_keyboard, send_message_chunks, send_stale_button};

pub(in crate::bot::handlers) async fn show_gradebook(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let gradebook = match screen
        .services
        .courses
        .gradebook(screen.actor, course_id)
        .await
    {
        Ok(gradebook) => gradebook,
        Err(error) => return send_app_error(screen.bot, screen.chat_id, error).await,
    };

    if gradebook.students.is_empty() {
        screen
            .bot
            .send_message(screen.chat_id, "В группе пока нет студентов.")
            .await?;
        return Ok(());
    }

    let message = screen
        .bot
        .send_message(screen.chat_id, "📊 Оценки курса. Выберите студента:")
        .reply_markup(gradebook_students_keyboard(course_id, &gradebook.students))
        .await?;
    screen
        .dialogue
        .update(BotState::Flow(FlowState::ChoosingStudent {
            message_id: message.id,
            purpose: StudentChoice::Grades { course_id },
            student_ids: gradebook
                .students
                .iter()
                .map(|student| student.student_id)
                .collect(),
        }))
        .await?;
    Ok(())
}

pub(super) async fn show_student_grades(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> HandlerResult {
    let gradebook = match screen
        .services
        .courses
        .gradebook(screen.actor, course_id)
        .await
    {
        Ok(gradebook) => gradebook,
        Err(error) => return send_app_error(screen.bot, screen.chat_id, error).await,
    };

    let Some(student) = gradebook
        .students
        .iter()
        .find(|student| student.student_id == student_id)
    else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    let mut text = format!(
        "👤 {}\n🏅 Итого: {} из {}\n",
        student.display_name,
        student.total(),
        student.max_total(),
    );
    if student.grades.is_empty() {
        text.push_str("\nВ курсе пока нет оценочных заданий.");
    }
    for grade in &student.grades {
        // Непроверенная сдача и выставленный ноль - разные вещи
        let score = grade
            .final_score
            .map_or_else(|| "—".to_owned(), |score| score.to_string());
        text.push_str(&format!(
            "\n{} · {} из {}",
            grade.title, score, grade.max_score
        ));
    }

    send_message_chunks(screen.bot, screen.chat_id, text, None).await?;
    show_gradebook(screen, course_id).await
}
