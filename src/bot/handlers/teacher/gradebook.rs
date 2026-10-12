use teloxide::payloads::SendMessageSetters;

use crate::app::{CourseStudent, StudentGradesCard};
use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, FlowState, ScreenState, StudentChoice, StudentList};
use crate::bot::texts;
use crate::bot::views::{
    gradebook_groups_keyboard, gradebook_students_keyboard, paginated_keyboard, send_screen,
    student_grades_keyboard,
};
use crate::domain::{StudentGroupId, TeachingCourseId, UserId};
use crate::pagination::{Page, PageRequest};

pub(crate) async fn show_gradebook_groups(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let course = screen
        .services()
        .courses
        .find_for_teacher(screen.actor(), course_id)
        .await?;
    if let [group] = course.groups.as_slice() {
        return show_gradebook_page(screen, course_id, group.id(), PageRequest::default()).await;
    }

    screen
        .bot()
        .send_message(screen.chat_id(), texts::GRADEBOOK_CHOOSE_GROUP)
        .reply_markup(gradebook_groups_keyboard(course_id, &course.groups))
        .await?;
    Ok(())
}

pub(crate) async fn show_gradebook_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
    page: PageRequest,
) -> HandlerResult {
    let students = screen
        .services()
        .courses
        .list_students(screen.actor(), course_id, group_id, page)
        .await?;

    render_gradebook(screen, course_id, group_id, &students).await
}

async fn render_gradebook(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
    students: &Page<CourseStudent>,
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
            PageTarget::Gradebook(course_id, group_id),
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
    list: Option<StudentList>,
) -> HandlerResult {
    let card = screen
        .services()
        .courses
        .student_grades_card(screen.actor(), course_id, student_id, page)
        .await?;
    send_student_grades(screen, course_id, card, list).await
}

pub(crate) async fn send_student_grades(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    card: StudentGradesCard,
    list: Option<StudentList>,
) -> HandlerResult {
    let StudentGradesCard {
        grades: student,
        group_id,
        makeup,
    } = card;
    let student_id = student.student_id;

    let mut lines = vec![texts::grades_header(
        &student.display_name,
        student.work_total(),
        student.penalty_total(),
        student.total(),
        student.max_total(),
    )];
    if let Some(pool) = makeup {
        lines.push(texts::gradebook_makeup_attempts(
            pool.balance.remaining(),
            pool.balance.allowed(),
        ));
    }
    lines.push(String::new());
    if student.grades.is_empty() {
        lines.push(texts::NO_SCORED_ASSIGNMENTS.to_owned());
    }
    for grade in &student.grades {
        let score = grade
            .score
            .map_or_else(|| "—".to_owned(), |score| score.work_score.to_string());
        lines.push(texts::grade_entry(
            &grade.title,
            &score,
            grade.max_score,
            grade.score.map_or(0, |score| score.course_total_penalty),
            grade.score.is_some_and(|score| score.makeup),
        ));
    }
    let text = lines.join("\n");

    let keyboard = paginated_keyboard(
        student_grades_keyboard(course_id, group_id, student_id, makeup.is_some()),
        PageTarget::StudentGrades(course_id, student_id),
        student.grade_page,
        student.next_grade_page,
    );
    let message_id = send_screen(screen.bot(), screen.chat_id(), text, keyboard).await?;
    screen
        .dialogue()
        .update(BotState::Screen(ScreenState::StudentGrades {
            message_id,
            course_id,
            student_id,
            list,
        }))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::bot::texts;

    #[test]
    fn the_card_shows_sums_and_penalties_only_when_some_go_to_the_total() {
        let with_penalties = texts::grades_header("Student", 14, 4, 10, 20);
        let without_penalties = texts::grades_header("Student", 14, 0, 14, 20);

        assert!(with_penalties.contains("14"), "{with_penalties}");
        assert!(with_penalties.contains("−4"), "{with_penalties}");
        assert!(with_penalties.contains("10 из 20"), "{with_penalties}");
        assert!(
            without_penalties.contains("14 из 20"),
            "{without_penalties}"
        );
        assert!(!without_penalties.contains('−'), "{without_penalties}");
        assert!(without_penalties.lines().count() < with_penalties.lines().count());
    }

    #[test]
    fn an_assignment_line_names_only_a_penalty_to_the_total() {
        let to_total = texts::grade_entry("Ownership", "8", 10, 4, false);
        let to_work = texts::grade_entry("Traits", "4", 10, 0, false);

        assert!(to_total.contains("8 из 10"), "{to_total}");
        assert!(to_total.contains("−4"), "{to_total}");
        assert!(to_work.contains("4 из 10"), "{to_work}");
        assert!(!to_work.contains('−'), "{to_work}");
    }
}
