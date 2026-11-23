use std::collections::BTreeSet;

use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::types::MessageId;

use super::home::send_home;
use super::input::require_text_within;
use super::reset_dialogue;
use super::teacher::detach;
use crate::app::{
    Actor, AppError, CourseInvitation, ErrorCode, OfferedCourse, RegisterStudent, RegistrationOffer,
};
use crate::bot::HandlerResult;
use crate::bot::callbacks::FlowAction;
use crate::bot::context::{ActorChat, Chat, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{
    ask_input, registration_groups_keyboard, send_message_chunks, subjects_keyboard,
};
use crate::domain::{
    DISPLAY_NAME_MAX_LEN, DomainError, StudentGroup, StudentGroupId, TeachingCourseId,
    TelegramUserId, User,
};
use crate::invites::VerifiedInvite;

pub(super) async fn start_registration(
    chat: Chat<'_>,
    telegram_user_id: TelegramUserId,
    payload: &str,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let services = chat.services();
    let invites = chat.invites();
    match services.users.find_user(telegram_user_id).await {
        Ok(user) => {
            send_home(chat, &user).await?;
            return offer_enrollment(chat, &user, payload).await;
        }
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if payload.is_empty() {
        return super::home::open_guest(chat).await;
    }
    // Гость не должен отличать ошибку подписи от отзыва ссылки по ответу бота.
    let Ok(invite) = invites.decode(payload) else {
        return send_error_code(bot, chat_id, ErrorCode::InviteInvalid).await;
    };
    let CourseInvitation { subject, groups } = services.users.resolve_course_invite(invite).await?;
    if let [group] = groups.as_slice() {
        return ask_subjects(chat, invite, group.id()).await;
    }

    let sent = bot
        .send_message(
            chat_id,
            texts::registration_group_prompt(subject.name().as_str()),
        )
        .reply_markup(registration_groups_keyboard(&groups))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingRegistrationGroup {
            invite,
            message_id: sent.id,
        }))
        .await?;
    Ok(())
}

pub(super) async fn choose_group(
    chat: Chat<'_>,
    message_id: MessageId,
    group_id: StudentGroupId,
    state: &BotState,
) -> HandlerResult {
    let Some(invite) = state.choosing_registration_group(message_id) else {
        return chat.stale().await;
    };
    let invitation = chat.services().users.resolve_course_invite(invite).await;
    if let Err(error) = &invitation
        && error.code() == ErrorCode::InviteInvalid
    {
        reset_dialogue(chat.dialogue()).await?;
    }
    let CourseInvitation { groups, .. } = invitation?;
    if !groups.iter().any(|group| group.id() == group_id) {
        return chat.stale().await;
    }
    chat.bot()
        .edit_message_reply_markup(chat.chat_id(), message_id)
        .await?;
    ask_subjects(chat, invite, group_id).await
}

async fn ask_subjects(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    group_id: StudentGroupId,
) -> HandlerResult {
    let offer = registration_offer(chat, invite, group_id).await?;
    if offer.courses.len() <= 1 {
        return ask_name(chat, invite, &offer.group, &offer.courses).await;
    }

    show_subjects(chat, invite, offer, BTreeSet::from([invite.course_id()])).await
}

async fn reoffer_subjects(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    group_id: StudentGroupId,
    selected: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    let offer = registration_offer(chat, invite, group_id).await?;
    chat.bot()
        .send_message(chat.chat_id(), texts::SUBJECTS_CHANGED)
        .await?;
    if offer.courses.is_empty() {
        return ask_name(chat, invite, &offer.group, &[]).await;
    }

    show_subjects(chat, invite, offer, selected).await
}

async fn show_subjects(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    RegistrationOffer { group, courses }: RegistrationOffer,
    mut selected: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    retain_offered(&mut selected, &courses);
    let sent = chat
        .bot()
        .send_message(
            chat.chat_id(),
            texts::registration_subjects_prompt(group.code().as_str()),
        )
        .reply_markup(registration_subjects_keyboard(&courses, &selected))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingRegistrationSubjects {
            invite,
            group_id: group.id(),
            message_id: sent.id,
            selected,
        }))
        .await?;
    Ok(())
}

pub(super) async fn toggle_registration_subject(
    chat: Chat<'_>,
    message_id: MessageId,
    course_id: TeachingCourseId,
    state: BotState,
) -> HandlerResult {
    let Some((invite, group_id, mut selected)) = state.choosing_registration_subjects(message_id)
    else {
        return chat.stale().await;
    };
    let RegistrationOffer { courses, .. } = registration_offer(chat, invite, group_id).await?;
    if !toggle(&courses, &mut selected, course_id) {
        return chat.stale().await;
    }
    chat.bot()
        .edit_message_reply_markup(chat.chat_id(), message_id)
        .reply_markup(registration_subjects_keyboard(&courses, &selected))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingRegistrationSubjects {
            invite,
            group_id,
            message_id,
            selected,
        }))
        .await?;
    Ok(())
}

pub(super) async fn finish_registration_subjects(
    chat: Chat<'_>,
    message_id: MessageId,
    state: BotState,
) -> HandlerResult {
    let Some((invite, group_id, selected)) = state.choosing_registration_subjects(message_id)
    else {
        return chat.stale().await;
    };
    if selected.is_empty() {
        chat.bot()
            .send_message(chat.chat_id(), texts::SUBJECTS_REQUIRED)
            .await?;
        return Ok(());
    }
    let RegistrationOffer { group, courses } = registration_offer(chat, invite, group_id).await?;
    let chosen: Vec<_> = courses
        .into_iter()
        .filter(|course| selected.contains(&course.course_id))
        .collect();
    if chosen.len() != selected.len() {
        detach(chat.bot(), chat.chat_id(), Some(message_id)).await?;
        return reoffer_subjects(chat, invite, group_id, selected).await;
    }
    chat.bot()
        .edit_message_reply_markup(chat.chat_id(), message_id)
        .await?;
    ask_name(chat, invite, &group, &chosen).await
}

async fn registration_offer(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    group_id: StudentGroupId,
) -> HandlerResult<RegistrationOffer> {
    let offer = chat
        .services()
        .users
        .registration_offer(invite, group_id)
        .await;
    if let Err(error) = &offer
        && error.code() == ErrorCode::InviteInvalid
    {
        reset_dialogue(chat.dialogue()).await?;
    }
    Ok(offer?)
}

fn registration_subjects_keyboard(
    courses: &[OfferedCourse],
    selected: &BTreeSet<TeachingCourseId>,
) -> teloxide::types::InlineKeyboardMarkup {
    subjects_keyboard(
        courses,
        selected,
        FlowAction::RegistrationSubject,
        FlowAction::RegistrationSubjectsDone,
    )
}

fn enrollment_subjects_keyboard(
    courses: &[OfferedCourse],
    selected: &BTreeSet<TeachingCourseId>,
) -> teloxide::types::InlineKeyboardMarkup {
    subjects_keyboard(
        courses,
        selected,
        FlowAction::EnrollSubject,
        FlowAction::EnrollSubjectsDone,
    )
}

fn toggle(
    courses: &[OfferedCourse],
    selected: &mut BTreeSet<TeachingCourseId>,
    course_id: TeachingCourseId,
) -> bool {
    if !courses.iter().any(|course| course.course_id == course_id) {
        return false;
    }
    retain_offered(selected, courses);
    if !selected.remove(&course_id) {
        selected.insert(course_id);
    }
    true
}

fn retain_offered(selected: &mut BTreeSet<TeachingCourseId>, courses: &[OfferedCourse]) {
    selected.retain(|id| courses.iter().any(|course| course.course_id == *id));
}

fn subject_names(courses: &[OfferedCourse]) -> Vec<&str> {
    courses
        .iter()
        .map(|course| course.subject.name().as_str())
        .collect()
}

async fn ask_name(
    chat: Chat<'_>,
    invite: VerifiedInvite,
    group: &StudentGroup,
    courses: &[OfferedCourse],
) -> HandlerResult {
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
            group_id: group.id(),
            courses: courses.iter().map(|course| course.course_id).collect(),
        }))
        .await?;
    ask_input(
        chat.bot(),
        chat.chat_id(),
        &texts::registration_prompt(group.code().as_str(), &subject_names(courses)),
    )
    .await
}

pub(super) async fn submit_name(
    step: TextStep<'_>,
    invite: VerifiedInvite,
    group_id: StudentGroupId,
    courses: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let telegram_user_id = step.telegram_user_id();
    let dialogue = step.dialogue();
    let services = step.services();

    let Some(text) =
        require_text_within(bot, message, DISPLAY_NAME_MAX_LEN, texts::FIELD_NAME).await?
    else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::NAME_REQUIRED).await?;
        return Ok(());
    }

    let registered = services
        .users
        .register_student(RegisterStudent {
            telegram_user_id,
            display_name: text.trim().to_owned(),
            invite,
            student_group_id: Some(group_id),
            courses: Some(courses.clone()),
            registered_at: step.now(),
        })
        .await;
    if matches!(
        registered,
        Err(AppError::Domain(
            DomainError::CourseNotOfferedToStudentGroup
        ))
    ) {
        return reoffer_subjects(step.chat(), invite, group_id, courses).await;
    }
    if let Err(error) = &registered
        && matches!(
            error.code(),
            ErrorCode::InviteInvalid | ErrorCode::AlreadyRegistered
        )
    {
        reset_dialogue(dialogue).await?;
    }
    let registered = registered?;
    reset_dialogue(dialogue).await?;
    bot.send_message(chat_id, texts::REGISTRATION_COMPLETE)
        .await?;
    send_home(step.chat(), &registered.user).await
}

async fn offer_enrollment(chat: Chat<'_>, user: &User, payload: &str) -> HandlerResult {
    let Ok(invite) = chat.invites().decode(payload) else {
        return Ok(());
    };
    let offer = chat
        .services()
        .users
        .enrollment_offer(Actor::new(user.id()), invite)
        .await?;
    let Some(courses) = offer else {
        return Ok(());
    };

    show_enrollment(chat, &courses, BTreeSet::from([invite.course_id()])).await
}

async fn show_enrollment(
    chat: Chat<'_>,
    courses: &[OfferedCourse],
    selected: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    let sent = chat
        .bot()
        .send_message(chat.chat_id(), texts::ENROLLMENT_PROMPT)
        .reply_markup(enrollment_subjects_keyboard(courses, &selected))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingSubjects {
            message_id: sent.id,
            selected,
        }))
        .await?;
    Ok(())
}

async fn refresh_enrollment(
    screen: ActorChat<'_>,
    message_id: MessageId,
    mut selected: BTreeSet<TeachingCourseId>,
) -> HandlerResult {
    let courses = screen.services().users.open_courses(screen.actor()).await?;
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    reset_dialogue(screen.dialogue()).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::SUBJECTS_CHANGED)
        .await?;
    if courses.is_empty() {
        return Ok(());
    }
    retain_offered(&mut selected, &courses);
    show_enrollment(screen.chat(), &courses, selected).await
}

pub(super) async fn toggle_enrollment_subject(
    screen: ActorChat<'_>,
    message_id: MessageId,
    course_id: TeachingCourseId,
    state: BotState,
) -> HandlerResult {
    let Some(mut selected) = state.choosing_subjects(message_id) else {
        return screen.stale().await;
    };
    let courses = screen.services().users.open_courses(screen.actor()).await?;
    if !toggle(&courses, &mut selected, course_id) {
        return screen.stale().await;
    }
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(enrollment_subjects_keyboard(&courses, &selected))
        .await?;
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::ChoosingSubjects {
            message_id,
            selected,
        }))
        .await?;
    Ok(())
}

pub(super) async fn finish_enrollment(
    screen: ActorChat<'_>,
    message_id: MessageId,
    state: BotState,
) -> HandlerResult {
    let Some(selected) = state.choosing_subjects(message_id) else {
        return screen.stale().await;
    };
    if selected.is_empty() {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::SUBJECTS_REQUIRED)
            .await?;
        return Ok(());
    }

    let enrolled = screen
        .services()
        .users
        .enroll(screen.actor(), &selected)
        .await;
    if matches!(
        enrolled,
        Err(AppError::Domain(
            DomainError::CourseNotOfferedToStudentGroup
        ))
    ) {
        return refresh_enrollment(screen, message_id, selected).await;
    }
    let enrolled = enrolled?;
    reset_dialogue(screen.dialogue()).await?;
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .await?;
    send_message_chunks(
        screen.bot(),
        screen.chat_id(),
        &texts::enrollment_complete(&subject_names(&enrolled)),
        None,
    )
    .await
}
