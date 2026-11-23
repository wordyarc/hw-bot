use std::collections::BTreeSet;

use teloxide::payloads::{EditMessageReplyMarkupSetters, EditMessageTextSetters};
use teloxide::types::MessageId;

use super::{end_on_refusal, finish, set_flow};
use crate::app::CreateCourse;
use crate::bot::HandlerResult;
use crate::bot::context::ActorChat;
use crate::bot::handlers::reset_dialogue;
use crate::bot::handlers::teacher::detach;
use crate::bot::state::{AdminFlow, BotState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{
    admin_add_group_keyboard, admin_close_keyboard, admin_course_groups_keyboard,
    admin_course_keyboard, admin_course_subjects_keyboard, admin_course_teachers_keyboard,
    admin_courses_keyboard, send_screen,
};
use crate::domain::{StudentGroupId, SubjectId, TeachingCourseId, UserId};
use crate::pagination::PageRequest;

pub(super) struct CourseDraft {
    pub subject_id: SubjectId,
    pub group_ids: BTreeSet<StudentGroupId>,
    pub selected: BTreeSet<UserId>,
}

pub(super) async fn show_courses(screen: ActorChat<'_>, closed: bool, page: u32) -> HandlerResult {
    let courses = screen
        .services()
        .admin
        .list_courses(screen.actor(), closed, PageRequest::new(page))
        .await?;
    let text = match (closed, courses.items.is_empty()) {
        (false, false) => texts::ADMIN_COURSES,
        (false, true) => texts::ADMIN_NO_COURSES,
        (true, false) => texts::ADMIN_CLOSED_COURSES,
        (true, true) => texts::ADMIN_NO_CLOSED_COURSES,
    };
    send_screen(
        screen.bot(),
        screen.chat_id(),
        text,
        admin_courses_keyboard(&courses, closed),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn show_course(
    screen: ActorChat<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let card = screen
        .services()
        .admin
        .course_card(screen.actor(), course_id)
        .await?;
    let groups: Vec<&str> = card
        .details
        .groups
        .iter()
        .map(|group| group.code().as_str())
        .collect();
    let teachers: Vec<&str> = card
        .teachers
        .iter()
        .map(|teacher| teacher.display_name().as_str())
        .collect();
    let text = texts::admin_course_card(
        card.details.subject.name().as_str(),
        &groups,
        &teachers,
        card.details.course.is_closed(),
        card.sandbox,
    );
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_course_keyboard(&card),
    )
    .await?;
    screen
        .dialogue()
        .update(BotState::Screen(ScreenState::AdminCourse {
            message_id,
            course_id,
        }))
        .await?;
    Ok(())
}

pub(super) async fn assign_self(
    screen: ActorChat<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(course_id) = state.admin_course(message_id) else {
        return screen.stale().await;
    };
    screen
        .services()
        .admin
        .assign_teacher(screen.actor(), screen.actor().user_id(), course_id)
        .await?;
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ADMIN_SELF_ASSIGNED)
        .await?;
    show_course(screen, course_id).await
}

pub(super) async fn reopen(
    screen: ActorChat<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some(course_id) = state.admin_course(message_id) else {
        return screen.stale().await;
    };
    screen
        .services()
        .admin
        .reopen_course(screen.actor(), course_id)
        .await?;
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ADMIN_COURSE_REOPENED)
        .await?;
    show_course(screen, course_id).await
}

pub(super) async fn choose_subject(screen: ActorChat<'_>, page: u32) -> HandlerResult {
    let subjects = screen
        .services()
        .admin
        .list_subjects(screen.actor(), PageRequest::new(page))
        .await?;
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_CHOOSE_COURSE_SUBJECT,
        admin_course_subjects_keyboard(&subjects),
    )
    .await?;
    set_flow(screen, AdminFlow::ChoosingCourseSubject { message_id }).await
}

pub(super) async fn start_course_groups(
    screen: ActorChat<'_>,
    message_id: MessageId,
    subject_id: SubjectId,
) -> HandlerResult {
    show_course_groups(screen, message_id, subject_id, BTreeSet::new(), 0).await
}

pub(super) async fn show_course_groups(
    screen: ActorChat<'_>,
    message_id: MessageId,
    subject_id: SubjectId,
    selected: BTreeSet<StudentGroupId>,
    page: u32,
) -> HandlerResult {
    let groups = screen
        .services()
        .admin
        .groups_free_for_subject(screen.actor(), subject_id, PageRequest::new(page))
        .await?;
    screen
        .bot()
        .edit_message_text(
            screen.chat_id(),
            message_id,
            texts::ADMIN_CHOOSE_COURSE_GROUPS,
        )
        .reply_markup(admin_course_groups_keyboard(&groups, &selected))
        .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingCourseGroups {
            message_id,
            subject_id,
            selected,
            page,
        },
    )
    .await
}

pub(super) async fn toggle_course_group(
    screen: ActorChat<'_>,
    message_id: MessageId,
    subject_id: SubjectId,
    mut selected: BTreeSet<StudentGroupId>,
    page: u32,
    group_id: StudentGroupId,
) -> HandlerResult {
    let groups = screen
        .services()
        .admin
        .groups_free_for_subject(screen.actor(), subject_id, PageRequest::new(page))
        .await?;
    if !groups.items.iter().any(|group| group.id() == group_id) {
        return screen.stale().await;
    }
    if !selected.remove(&group_id) {
        selected.insert(group_id);
    }
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(admin_course_groups_keyboard(&groups, &selected))
        .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingCourseGroups {
            message_id,
            subject_id,
            selected,
            page,
        },
    )
    .await
}

pub(super) async fn finish_course_groups(
    screen: ActorChat<'_>,
    message_id: MessageId,
    subject_id: SubjectId,
    group_ids: BTreeSet<StudentGroupId>,
) -> HandlerResult {
    if group_ids.is_empty() {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::ADMIN_CHOOSE_AT_LEAST_ONE)
            .await?;
        return Ok(());
    }
    let draft = CourseDraft {
        subject_id,
        group_ids,
        selected: BTreeSet::new(),
    };
    show_course_teachers(screen, message_id, draft, 0).await
}

pub(super) async fn show_course_teachers(
    screen: ActorChat<'_>,
    message_id: MessageId,
    draft: CourseDraft,
    page: u32,
) -> HandlerResult {
    let staff = screen
        .services()
        .admin
        .list_staff(screen.actor(), PageRequest::new(page))
        .await?;
    screen
        .bot()
        .edit_message_text(
            screen.chat_id(),
            message_id,
            texts::ADMIN_CHOOSE_COURSE_TEACHERS,
        )
        .reply_markup(admin_course_teachers_keyboard(&staff, &draft.selected))
        .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingCourseTeachers {
            message_id,
            subject_id: draft.subject_id,
            group_ids: draft.group_ids,
            selected: draft.selected,
            page,
        },
    )
    .await
}

pub(super) async fn toggle_course_teacher(
    screen: ActorChat<'_>,
    message_id: MessageId,
    mut draft: CourseDraft,
    page: u32,
    teacher_id: UserId,
) -> HandlerResult {
    let staff = screen
        .services()
        .admin
        .list_staff(screen.actor(), PageRequest::new(page))
        .await?;
    if !staff.items.iter().any(|user| user.id() == teacher_id) {
        return screen.stale().await;
    }
    if !draft.selected.remove(&teacher_id) {
        draft.selected.insert(teacher_id);
    }
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(admin_course_teachers_keyboard(&staff, &draft.selected))
        .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingCourseTeachers {
            message_id,
            subject_id: draft.subject_id,
            group_ids: draft.group_ids,
            selected: draft.selected,
            page,
        },
    )
    .await
}

pub(super) async fn create_course(
    screen: ActorChat<'_>,
    message_id: MessageId,
    draft: CourseDraft,
) -> HandlerResult {
    if draft.selected.is_empty() {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::ADMIN_CHOOSE_AT_LEAST_ONE)
            .await?;
        return Ok(());
    }
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let created = screen
        .services()
        .admin
        .create_course(CreateCourse {
            actor: screen.actor(),
            subject_id: draft.subject_id,
            group_ids: draft.group_ids,
            teacher_ids: draft.selected,
            created_at: screen.now(),
        })
        .await;
    let Some(course) = end_on_refusal(screen, created).await? else {
        return show_courses(screen, false, 0).await;
    };
    finish(screen, texts::ADMIN_COURSE_CREATED).await?;
    show_course(screen, course.id()).await
}

pub(super) async fn choose_group_to_add(
    screen: ActorChat<'_>,
    course_id: TeachingCourseId,
    page: u32,
) -> HandlerResult {
    let admin = &screen.services().admin;
    let card = admin.course_card(screen.actor(), course_id).await?;
    card.details
        .course
        .ensure_open()
        .map_err(crate::app::AppError::from)?;
    let groups = admin
        .groups_free_for_subject(
            screen.actor(),
            card.details.subject.id(),
            PageRequest::new(page),
        )
        .await?;
    let text = if groups.items.is_empty() {
        texts::ADMIN_NO_FREE_GROUPS
    } else {
        texts::ADMIN_CHOOSE_GROUP_TO_ADD
    };
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        text,
        admin_add_group_keyboard(course_id, &groups),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingGroupToAdd {
            message_id,
            course_id,
        },
    )
    .await
}

pub(super) async fn add_group(
    screen: ActorChat<'_>,
    message_id: MessageId,
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let added = screen
        .services()
        .admin
        .add_course_group(screen.actor(), course_id, group_id)
        .await;
    if end_on_refusal(screen, added).await?.is_some() {
        finish(screen, texts::ADMIN_GROUP_ADDED).await?;
    }
    show_course(screen, course_id).await
}

pub(super) async fn ask_close(screen: ActorChat<'_>, course_id: TeachingCourseId) -> HandlerResult {
    let preview = screen
        .services()
        .admin
        .close_preview(screen.actor(), course_id)
        .await?;
    let text = texts::admin_close_confirmation(
        preview.details.subject.name().as_str(),
        preview.published,
        preview.pending,
    );
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_close_keyboard(course_id),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ConfirmingCourseClose {
            message_id,
            course_id,
            published: preview.published,
            pending: preview.pending,
        },
    )
    .await
}

pub(super) async fn confirm_close(
    screen: ActorChat<'_>,
    message_id: MessageId,
    course_id: TeachingCourseId,
    published: u32,
    pending: u32,
) -> HandlerResult {
    let admin = &screen.services().admin;
    let preview = admin.close_preview(screen.actor(), course_id).await;
    let Some(preview) = end_on_refusal(screen, preview).await? else {
        detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
        return show_course(screen, course_id).await;
    };
    if (preview.published, preview.pending) != (published, pending) {
        let text = format!(
            "{}\n\n{}",
            texts::ADMIN_CLOSE_NUMBERS_CHANGED,
            texts::admin_close_confirmation(
                preview.details.subject.name().as_str(),
                preview.published,
                preview.pending,
            )
        );
        screen
            .bot()
            .edit_message_text(screen.chat_id(), message_id, text)
            .reply_markup(admin_close_keyboard(course_id))
            .await?;
        return set_flow(
            screen,
            AdminFlow::ConfirmingCourseClose {
                message_id,
                course_id,
                published: preview.published,
                pending: preview.pending,
            },
        )
        .await;
    }

    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let closed = admin
        .close_course(screen.actor(), course_id, screen.now())
        .await;
    if let Some(closed) = end_on_refusal(screen, closed).await? {
        finish(
            screen,
            &texts::admin_course_closed(closed.archived, closed.pending),
        )
        .await?;
    }
    show_course(screen, course_id).await
}
