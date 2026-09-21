use teloxide::payloads::SendMessageSetters;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, FlowState, StudentChoice};
use crate::bot::views::paginated_keyboard;
use crate::bot::views::{gradebook_students_keyboard, send_message_chunks};
use crate::domain::{TeachingCourseId, UserId};
use crate::pagination::{Page, PageRequest};

pub(in crate::bot::handlers) async fn show_gradebook_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> HandlerResult {
    let students = screen
        .deps
        .context
        .services
        .courses
        .list_students(screen.actor, course_id, page)
        .await?;

    render_gradebook(screen, course_id, &students).await
}

async fn render_gradebook(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    students: &Page<crate::app::CourseStudent>,
) -> HandlerResult {
    if students.items.is_empty() && students.page == 0 {
        screen
            .bot
            .send_message(screen.chat_id, "В группе пока нет студентов.")
            .await?;
        return Ok(());
    }

    let message = screen
        .bot
        .send_message(screen.chat_id, "📊 Оценки курса. Выберите студента:")
        .reply_markup(paginated_keyboard(
            gradebook_students_keyboard(course_id, &students.items),
            PageTarget::Gradebook(course_id),
            students.page,
            students.next_page,
        ))
        .await?;
    screen
        .deps
        .dialogue
        .update(BotState::Flow(FlowState::ChoosingStudent {
            message_id: message.id,
            purpose: StudentChoice::Grades { course_id },
            student_ids: students
                .items
                .iter()
                .map(|student| student.student_id)
                .collect(),
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn show_student_grades_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    student_id: UserId,
    page: PageRequest,
) -> HandlerResult {
    let student = screen
        .deps
        .context
        .services
        .courses
        .student_grades(screen.actor, course_id, student_id, page)
        .await?;

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

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "⬅️ К студентам курса",
        PageTarget::Gradebook(course_id).callback(0),
    )]]);
    send_message_chunks(
        screen.bot,
        screen.chat_id,
        text,
        Some(paginated_keyboard(
            keyboard,
            PageTarget::StudentGrades(course_id, student_id),
            student.grade_page,
            student.next_grade_page,
        )),
    )
    .await
}
