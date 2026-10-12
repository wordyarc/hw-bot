use crate::app::{CourseRoster, RosterGroup};
use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::Screen;
use crate::bot::handlers::reset_dialogue;
use crate::bot::texts;
use crate::bot::views::{list_text, paginated_keyboard, roster_keyboard, send_screen};
use crate::domain::TeachingCourseId;
use crate::pagination::PageRequest;

pub(crate) async fn show_roster_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> HandlerResult {
    let roster = screen
        .services()
        .courses
        .roster(screen.actor(), course_id, page)
        .await?;

    let text = roster_text(&roster);
    let keyboard = paginated_keyboard(
        roster_keyboard(course_id),
        PageTarget::Roster(course_id),
        roster.page,
        roster.next_page,
    );
    send_screen(screen.bot(), screen.chat_id(), text, keyboard).await?;
    reset_dialogue(screen.dialogue()).await
}

fn roster_text(roster: &CourseRoster) -> String {
    list_text(
        &texts::roster_header(roster.subject.name().as_str()),
        roster.groups.iter().map(group_section),
    )
}

fn group_section(group: &RosterGroup) -> String {
    let students = group.students.iter().map(|student| {
        texts::roster_student(
            student.position,
            &student.display_name,
            student.accepted,
            group.issued,
        )
    });
    std::iter::once(texts::roster_group_header(&group.code, group.size))
        .chain(students)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RosterStudent;
    use crate::test_support::subject;

    fn group(code: &str, issued: u32, students: &[(&str, u32)]) -> RosterGroup {
        RosterGroup {
            code: code.to_owned(),
            size: students.len() as u32,
            issued,
            students: students
                .iter()
                .zip(1..)
                .map(|(&(name, accepted), position)| RosterStudent {
                    position,
                    display_name: name.to_owned(),
                    accepted,
                })
                .collect(),
        }
    }

    #[test]
    fn every_group_is_a_section_under_its_header_with_progress_only_where_assignments_exist() {
        let roster = CourseRoster {
            subject: subject("Rust"),
            groups: vec![
                group("ИВТ-21", 3, &[("Анна", 1), ("Борис", 3)]),
                group("ИВТ-22", 0, &[("Вера", 0)]),
                group("ИВТ-23", 2, &[]),
            ],
            page: 0,
            next_page: None,
        };

        let sections: Vec<String> = roster_text(&roster)
            .split("\n\n")
            .map(str::to_owned)
            .collect();

        assert_eq!(
            sections,
            [
                texts::roster_header("Rust"),
                [
                    texts::roster_group_header("ИВТ-21", 2),
                    texts::roster_student(1, "Анна", 1, 3),
                    texts::roster_student(2, "Борис", 3, 3),
                ]
                .join("\n"),
                [
                    texts::roster_group_header("ИВТ-22", 1),
                    texts::roster_student(1, "Вера", 0, 0),
                ]
                .join("\n"),
                texts::roster_group_header("ИВТ-23", 0),
            ]
        );
    }
}
