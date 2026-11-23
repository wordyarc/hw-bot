use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::types::{MessageId, UsersShared};

use super::{finish, set_flow};
use crate::app::{ErrorCode, RegisterTeacher};
use crate::bot::HandlerResult;
use crate::bot::context::ActorChat;
use crate::bot::handlers::reset_dialogue;
use crate::bot::handlers::teacher::detach;
use crate::bot::state::{AdminFlow, BotState, ScreenState};
use crate::bot::texts;
use crate::bot::views::course_label;
use crate::bot::views::{
    TEACHER_REQUEST_ID, admin_keep_name_keyboard, admin_teacher_courses_keyboard,
    admin_teacher_keyboard, admin_teacher_pick_keyboard, admin_teachers_keyboard, send_screen,
};
use crate::domain::{TeachingCourseId, TelegramUserId, UserId};
use crate::pagination::PageRequest;

pub(super) async fn show_list(screen: ActorChat<'_>, page: u32) -> HandlerResult {
    let staff = screen
        .services()
        .admin
        .list_staff(screen.actor(), PageRequest::new(page))
        .await?;
    send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_TEACHERS,
        admin_teachers_keyboard(&staff),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn show_card(screen: ActorChat<'_>, teacher_id: UserId) -> HandlerResult {
    let card = screen
        .services()
        .admin
        .teacher_card(screen.actor(), teacher_id, PageRequest::default())
        .await?;
    let courses: Vec<String> = card
        .courses
        .items
        .iter()
        .map(|course| format!("• {}", course_label(course)))
        .collect();
    let text = texts::admin_teacher_card(
        card.teacher.display_name().as_str(),
        texts::role_name(card.teacher.role()),
        &courses,
    );
    send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_teacher_keyboard(teacher_id),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn ask_user(screen: ActorChat<'_>) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    set_flow(screen, AdminFlow::AwaitingTeacherUser).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::ADMIN_TEACHER_USER_PROMPT)
        .reply_markup(admin_teacher_pick_keyboard())
        .await?;
    Ok(())
}

pub(super) async fn submit_user_id(screen: ActorChat<'_>, text: &str) -> HandlerResult {
    let telegram_user_id = text
        .trim()
        .parse()
        .ok()
        .and_then(|id| TelegramUserId::new(id).ok());
    let Some(telegram_user_id) = telegram_user_id else {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::ADMIN_TEACHER_ID_INVALID)
            .await?;
        return Ok(());
    };
    ask_name(screen, telegram_user_id, None).await
}

pub(super) async fn user_shared(screen: ActorChat<'_>, shared: &UsersShared) -> HandlerResult {
    let user = match shared.users.as_slice() {
        [user] if shared.request_id.0 == TEACHER_REQUEST_ID => user,
        _ => return Ok(()),
    };
    let telegram_user_id = i64::try_from(user.user_id.0)
        .ok()
        .and_then(|id| TelegramUserId::new(id).ok());
    let Some(telegram_user_id) = telegram_user_id else {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::ADMIN_TEACHER_ID_INVALID)
            .await?;
        return Ok(());
    };
    let suggested = [user.first_name.as_deref(), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let suggested = (!suggested.trim().is_empty()).then_some(suggested);
    ask_name(screen, telegram_user_id, suggested).await
}

async fn ask_name(
    screen: ActorChat<'_>,
    telegram_user_id: TelegramUserId,
    suggested: Option<String>,
) -> HandlerResult {
    screen.ask(texts::ADMIN_TEACHER_NAME_PROMPT).await?;
    let message_id = match &suggested {
        Some(name) => Some(
            screen
                .bot()
                .send_message(screen.chat_id(), texts::ADMIN_SUGGESTED_NAME)
                .reply_markup(admin_keep_name_keyboard(name))
                .await?
                .id,
        ),
        None => None,
    };
    set_flow(
        screen,
        AdminFlow::AwaitingTeacherName {
            telegram_user_id,
            suggested,
            message_id,
        },
    )
    .await
}

pub(super) async fn register(
    screen: ActorChat<'_>,
    message_id: Option<MessageId>,
    telegram_user_id: TelegramUserId,
    display_name: String,
) -> HandlerResult {
    let registered = screen
        .services()
        .admin
        .register_teacher(RegisterTeacher {
            actor: screen.actor(),
            telegram_user_id,
            display_name,
            registered_at: screen.now(),
        })
        .await;
    match registered {
        Ok(teacher) => {
            detach(screen.bot(), screen.chat_id(), message_id).await?;
            finish(screen, texts::ADMIN_TEACHER_REGISTERED).await?;
            show_card(screen, teacher.id()).await
        }
        Err(error) if error.code() == ErrorCode::AlreadyRegistered => {
            detach(screen.bot(), screen.chat_id(), message_id).await?;
            finish(screen, texts::ADMIN_USER_ALREADY_REGISTERED).await?;
            show_list(screen, 0).await
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) async fn show_courses(
    screen: ActorChat<'_>,
    teacher_id: UserId,
    page: u32,
) -> HandlerResult {
    let admin = &screen.services().admin;
    admin
        .teacher_card(screen.actor(), teacher_id, PageRequest::default())
        .await?;
    let courses = admin
        .list_every_course(screen.actor(), PageRequest::new(page))
        .await?;
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_TEACHER_COURSES,
        admin_teacher_courses_keyboard(teacher_id, &courses),
    )
    .await?;
    screen
        .dialogue()
        .update(BotState::Screen(ScreenState::AdminTeacherCourses {
            message_id,
            teacher_id,
            page,
        }))
        .await?;
    Ok(())
}

pub(super) async fn toggle_course(
    screen: ActorChat<'_>,
    message_id: MessageId,
    course_id: TeachingCourseId,
    state: &BotState,
) -> HandlerResult {
    let Some((teacher_id, page)) = state.admin_teacher_courses(message_id) else {
        return screen.stale().await;
    };
    let admin = &screen.services().admin;
    let card = admin.course_card(screen.actor(), course_id).await?;
    if card.details.course.teacher_ids().contains(&teacher_id) {
        admin
            .unassign_teacher(screen.actor(), teacher_id, course_id)
            .await?;
    } else {
        admin
            .assign_teacher(screen.actor(), teacher_id, course_id)
            .await?;
    }
    let courses = admin
        .list_every_course(screen.actor(), PageRequest::new(page))
        .await?;
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(admin_teacher_courses_keyboard(teacher_id, &courses))
        .await?;
    Ok(())
}
