use teloxide::payloads::SendMessageSetters;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, FlowState, StudentChoice};
use crate::bot::texts;
use crate::bot::views::paginated_keyboard;
use crate::bot::views::{gradebook_students_keyboard, send_message_chunks};
use crate::domain::{TeachingCourseId, UserId};
use crate::pagination::{Page, PageRequest};

pub(crate) async fn show_gradebook_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> HandlerResult {
    let students = screen
        .services()
        .courses
        .list_students(screen.actor(), course_id, page)
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
            .bot()
            .send_message(screen.chat_id(), texts::NO_STUDENTS)
            .await?;
        return Ok(());
    }

    let message = screen
        .bot()
        .send_message(screen.chat_id(), texts::GRADEBOOK_CHOOSE_STUDENT)
        .reply_markup(paginated_keyboard(
            gradebook_students_keyboard(course_id, &students.items),
            PageTarget::Gradebook(course_id),
            students.page,
            students.next_page,
        ))
        .await?;
    screen
        .dialogue()
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

pub(crate) async fn show_student_grades_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    student_id: UserId,
    page: PageRequest,
) -> HandlerResult {
    let student = screen
        .services()
        .courses
        .student_grades(screen.actor(), course_id, student_id, page)
        .await?;

    let mut lines = vec![
        texts::grades_header(&student.display_name, student.total(), student.max_total()),
        String::new(),
    ];
    if student.grades.is_empty() {
        lines.push(texts::NO_SCORED_ASSIGNMENTS.to_owned());
    }
    for grade in &student.grades {
        let score = grade
            .final_score
            .map_or_else(|| "—".to_owned(), |score| score.to_string());
        lines.push(texts::grade_entry(&grade.title, &score, grade.max_score));
    }
    let text = lines.join("\n");

    let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        texts::BUTTON_BACK_TO_STUDENTS,
        PageTarget::Gradebook(course_id).callback(0),
    )]]);
    send_message_chunks(
        screen.bot(),
        screen.chat_id(),
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
