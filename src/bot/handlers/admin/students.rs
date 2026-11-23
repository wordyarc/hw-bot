use std::collections::BTreeSet;

use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::types::MessageId;

use super::{end_on_refusal, finish, set_flow};
use crate::app::{AppError, ChangeStudentGroup, StudentListItem};
use crate::bot::HandlerResult;
use crate::bot::context::ActorChat;
use crate::bot::handlers::reset_dialogue;
use crate::bot::handlers::teacher::detach;
use crate::bot::state::{AdminFlow, BotState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{
    admin_keyboard, admin_search_keyboard, admin_student_courses_keyboard,
    admin_student_groups_keyboard, admin_student_keyboard, admin_unenroll_keyboard, send_screen,
};
use crate::domain::{DomainError, StudentGroupId, TeachingCourseId, UserId};
use crate::pagination::{Page, PageRequest};

pub(super) async fn ask_query(screen: ActorChat<'_>) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    set_flow(screen, AdminFlow::AwaitingStudentQuery).await?;
    screen.ask(texts::ADMIN_STUDENT_QUERY_PROMPT).await
}

pub(super) async fn submit_query(screen: ActorChat<'_>, query: String) -> HandlerResult {
    let found = screen
        .services()
        .admin
        .search_students(screen.actor(), &query, PageRequest::default())
        .await?;
    reset_dialogue(screen.dialogue()).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ADMIN_SEARCH_DONE)
        .reply_markup(admin_keyboard())
        .await?;
    send_results(screen, query, found).await
}

pub(super) async fn show_search_page(
    screen: ActorChat<'_>,
    message_id: MessageId,
    page: u32,
    state: &BotState,
) -> HandlerResult {
    let Some(query) = state.admin_search(message_id) else {
        return screen.stale().await;
    };
    let query = query.to_owned();
    let found = screen
        .services()
        .admin
        .search_students(screen.actor(), &query, PageRequest::new(page))
        .await?;
    send_results(screen, query, found).await
}

async fn send_results(
    screen: ActorChat<'_>,
    query: String,
    found: Page<StudentListItem>,
) -> HandlerResult {
    let text = if found.items.is_empty() {
        texts::ADMIN_STUDENTS_NOT_FOUND
    } else {
        texts::ADMIN_STUDENTS_FOUND
    };
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        text,
        admin_search_keyboard(&found),
    )
    .await?;
    screen
        .dialogue()
        .update(BotState::Screen(ScreenState::AdminSearch {
            message_id,
            query,
        }))
        .await?;
    Ok(())
}

pub(super) async fn show_card(screen: ActorChat<'_>, student_id: UserId) -> HandlerResult {
    let card = screen
        .services()
        .admin
        .student_card(screen.actor(), student_id)
        .await?;
    let subjects: Vec<&str> = card
        .enrollments
        .iter()
        .map(|enrollment| enrollment.subject_name.as_str())
        .collect();
    let text = texts::admin_student_card(
        card.student.display_name().as_str(),
        card.group.code().as_str(),
        &subjects,
        card.blocking_submissions,
    );
    send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_student_keyboard(&card),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn ask_name(screen: ActorChat<'_>, student_id: UserId) -> HandlerResult {
    screen
        .services()
        .admin
        .student_card(screen.actor(), student_id)
        .await?;
    set_flow(screen, AdminFlow::AwaitingStudentName { student_id }).await?;
    screen.ask(texts::ADMIN_STUDENT_NAME_PROMPT).await
}

pub(super) async fn submit_name(
    screen: ActorChat<'_>,
    student_id: UserId,
    name: String,
) -> HandlerResult {
    screen
        .services()
        .admin
        .rename_student(screen.actor(), student_id, name)
        .await?;
    finish(screen, &texts::admin_student_renamed()).await?;
    show_card(screen, student_id).await
}

pub(super) async fn choose_new_group(
    screen: ActorChat<'_>,
    student_id: UserId,
    page: u32,
) -> HandlerResult {
    let admin = &screen.services().admin;
    let card = admin.student_card(screen.actor(), student_id).await?;
    if card.blocking_submissions > 0 {
        return Err(AppError::from(DomainError::StudentHasSubmissions).into());
    }
    let groups = admin
        .list_groups(screen.actor(), PageRequest::new(page))
        .await?;
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_CHOOSE_NEW_GROUP,
        admin_student_groups_keyboard(student_id, &groups, card.group.id()),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingStudentGroup {
            message_id,
            student_id,
        },
    )
    .await
}

pub(super) async fn choose_group(
    screen: ActorChat<'_>,
    message_id: MessageId,
    student_id: UserId,
    group_id: StudentGroupId,
) -> HandlerResult {
    let offer = screen
        .services()
        .admin
        .group_change_offer(screen.actor(), student_id, group_id)
        .await;
    let Some(offer) = end_on_refusal(screen, offer).await? else {
        return Ok(());
    };
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let text = texts::admin_choose_student_courses(
        offer.student.display_name().as_str(),
        offer.group.code().as_str(),
    );
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_student_courses_keyboard(&offer.courses, &offer.enrolled),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingStudentCourses {
            message_id,
            student_id,
            group_id,
            selected: offer.enrolled,
        },
    )
    .await
}

pub(super) async fn toggle_course(
    screen: ActorChat<'_>,
    message_id: MessageId,
    student_id: UserId,
    group_id: StudentGroupId,
    mut selected: BTreeSet<TeachingCourseId>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let offer = screen
        .services()
        .admin
        .group_change_offer(screen.actor(), student_id, group_id)
        .await;
    let Some(offer) = end_on_refusal(screen, offer).await? else {
        return Ok(());
    };
    if !offer
        .courses
        .iter()
        .any(|course| course.course_id == course_id)
    {
        return screen.stale().await;
    }
    if !selected.remove(&course_id) {
        selected.insert(course_id);
    }
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(admin_student_courses_keyboard(&offer.courses, &selected))
        .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingStudentCourses {
            message_id,
            student_id,
            group_id,
            selected,
        },
    )
    .await
}

pub(super) async fn finish_group_change(
    screen: ActorChat<'_>,
    message_id: MessageId,
    student_id: UserId,
    group_id: StudentGroupId,
    selected: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let changed = screen
        .services()
        .admin
        .change_student_group(ChangeStudentGroup {
            actor: screen.actor(),
            student_id,
            group_id,
            courses: selected,
        })
        .await;
    if end_on_refusal(screen, changed).await?.is_none() {
        return show_card(screen, student_id).await;
    }
    finish(screen, &texts::admin_student_moved()).await?;
    show_card(screen, student_id).await
}

pub(super) async fn ask_unenroll(
    screen: ActorChat<'_>,
    student_id: UserId,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let card = screen
        .services()
        .admin
        .student_card(screen.actor(), student_id)
        .await?;
    let Some(enrollment) = card
        .enrollments
        .iter()
        .find(|enrollment| enrollment.course_id == course_id)
    else {
        return screen.stale().await;
    };
    if enrollment.blocking_submissions > 0 {
        return Err(AppError::from(DomainError::StudentHasSubmissions).into());
    }
    let text = texts::admin_confirm_unenroll(
        card.student.display_name().as_str(),
        &enrollment.subject_name,
    );
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_unenroll_keyboard(student_id),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ConfirmingUnenroll {
            message_id,
            student_id,
            course_id,
        },
    )
    .await
}

pub(super) async fn confirm_unenroll(
    screen: ActorChat<'_>,
    message_id: MessageId,
    student_id: UserId,
    course_id: TeachingCourseId,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let removed = screen
        .services()
        .admin
        .unenroll_student(screen.actor(), student_id, course_id)
        .await;
    if end_on_refusal(screen, removed).await?.is_none() {
        return show_card(screen, student_id).await;
    }
    finish(screen, &texts::admin_student_unenrolled()).await?;
    show_card(screen, student_id).await
}
