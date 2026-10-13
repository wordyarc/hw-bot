use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::bot::HandlerResult;
use crate::bot::callbacks::PageTarget;
use crate::bot::context::Screen;
use crate::bot::handlers::reset_dialogue;
use crate::bot::state::{BotState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{
    assignment_actions_keyboard, course_assignments_keyboard, courses_keyboard, grading_lines,
    list_text, paginated_keyboard, send_message_chunks, teacher_menu,
};
use crate::domain::{AssignmentId, TeachingCourseId};
use crate::pagination::PageRequest;

pub(crate) async fn show_courses_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let courses = services.courses.list_for_teacher(actor, page).await?;
    if courses.items.is_empty() && page.page == 0 {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, texts::NO_COURSES)
            .reply_markup(teacher_menu(None))
            .await?;
        return Ok(());
    }

    bot.send_message(chat_id, texts::CHOOSE_COURSE)
        .reply_markup(paginated_keyboard(
            courses_keyboard(&courses.items),
            PageTarget::Courses,
            courses.page,
            courses.next_page,
        ))
        .await?;
    reset_dialogue(dialogue).await
}

pub(crate) async fn show_course_assignments_page(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let timezone = screen.timezone();
    let course = services.courses.find_for_teacher(actor, course_id).await?;
    let assignments = services
        .assignments
        .list_for_course(actor, course_id, page)
        .await?;
    let makeup = services.courses.active_makeup(actor, course_id).await?;

    let codes: Vec<&str> = course
        .groups
        .iter()
        .map(|group| group.code().as_str())
        .collect();
    let header = texts::course_header(course.subject.name().as_str(), &codes);
    let text = if assignments.items.is_empty() {
        format!("{header}\n\n{}", texts::NO_COURSE_ASSIGNMENTS)
    } else {
        let entries = assignments.items.iter().map(|assignment| {
            texts::course_assignment_entry(
                assignment.title().as_str(),
                texts::assignment_status(assignment.status()),
                &timezone.format(assignment.deadline_at()),
            )
        });
        format!(
            "{header}\n\n{}",
            list_text(texts::COURSE_ASSIGNMENTS_HEADER, entries)
        )
    };
    send_message_chunks(bot, chat_id, text, None).await?;

    bot.send_message(chat_id, texts::CHOOSE_ASSIGNMENT)
        .reply_markup(paginated_keyboard(
            course_assignments_keyboard(course_id, &assignments.items, makeup, timezone),
            PageTarget::Course(course_id),
            assignments.page,
            assignments.next_page,
        ))
        .await?;
    reset_dialogue(dialogue).await
}

pub(crate) async fn show_assignment(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let timezone = screen.timezone();
    let assignment = services
        .assignments
        .find_for_teacher(actor, assignment_id)
        .await?;
    let course = services
        .courses
        .find_for_teacher(actor, assignment.teaching_course_id())
        .await?;
    let groups = (course.groups.len() > 1).then(|| {
        let codes: Vec<&str> = course
            .groups
            .iter()
            .filter(|group| assignment.groups().contains(&group.id()))
            .map(|group| group.code().as_str())
            .collect();
        texts::groups_line(&codes)
    });
    let published = assignment
        .published_at()
        .map(|published_at| texts::published_line(&timezone.format_moment(published_at)));
    let text = texts::teacher_assignment_card(
        assignment.title().as_str(),
        texts::assignment_status(assignment.status()),
        groups.as_deref(),
        &grading_lines(&assignment, screen.now(), timezone),
        assignment.max_attempts(),
        published.as_deref(),
        assignment.body().as_str(),
    );
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, texts::CHOOSE_ACTION)
        .reply_markup(assignment_actions_keyboard(&assignment))
        .await?;
    dialogue
        .update(BotState::Screen(ScreenState::ManagingAssignment {
            message_id: message.id,
            assignment_id,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn publish_assignment(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if !state.is_managing_assignment(message_id, assignment_id) {
        return screen.stale().await;
    }
    screen
        .services()
        .assignments
        .publish(assignment_id, screen.actor(), screen.now())
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ASSIGNMENT_PUBLISHED)
        .await?;
    show_assignment(screen, assignment_id).await
}

pub(crate) async fn archive_assignment(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if !state.is_managing_assignment(message_id, assignment_id) {
        return screen.stale().await;
    }
    screen
        .services()
        .assignments
        .archive(assignment_id, screen.actor())
        .await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ASSIGNMENT_ARCHIVED)
        .await?;
    show_assignment(screen, assignment_id).await
}
