use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::app::{CourseInviteState, ErrorCode};
use crate::bot::HandlerResult;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, FlowState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{confirm_invite_rotation_keyboard, course_invite_keyboard};
use crate::domain::TeachingCourseId;

pub(crate) async fn show_course_invite(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let invite = screen
        .services()
        .courses
        .get_course_invite(screen.actor(), course_id)
        .await?;

    send_course_invite(screen, course_id, &invite).await
}

async fn send_course_invite(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    invite: &CourseInviteState,
) -> HandlerResult {
    let codes: Vec<&str> = invite
        .groups
        .iter()
        .map(|group| group.code().as_str())
        .collect();
    screen
        .bot()
        .send_message(
            screen.chat_id(),
            texts::invite_header(invite.subject.name().as_str(), &codes),
        )
        .await?;
    screen
        .bot()
        .send_message(
            screen.chat_id(),
            screen.invites().link(course_id, invite.generation),
        )
        .await?;
    let sent = screen
        .bot()
        .send_message(screen.chat_id(), texts::INVITE_REUSE_HINT)
        .reply_markup(course_invite_keyboard(course_id))
        .await?;

    screen
        .dialogue()
        .update(BotState::Screen(ScreenState::CourseInvite {
            message_id: sent.id,
            course_id,
            generation: invite.generation,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn ask_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, generation)) = state.course_invite(message_id) else {
        return screen.stale().await;
    };
    let sent = screen
        .bot()
        .send_message(screen.chat_id(), texts::INVITE_ROTATION_CONFIRM)
        .reply_markup(confirm_invite_rotation_keyboard())
        .await?;
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::ConfirmingInviteRotation {
            message_id: sent.id,
            course_id,
            generation,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn cancel_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, _)) = state.confirming_invite_rotation(message_id) else {
        return screen.stale().await;
    };
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .await?;
    show_course_invite(screen, course_id).await
}

pub(crate) async fn confirm_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, generation)) = state.confirming_invite_rotation(message_id) else {
        return screen.stale().await;
    };

    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .await?;

    match screen
        .services()
        .courses
        .rotate_course_invite(screen.actor(), course_id, generation)
        .await
    {
        Ok(invite) => {
            screen
                .bot()
                .send_message(screen.chat_id(), texts::INVITE_ROTATED)
                .await?;
            send_course_invite(screen, course_id, &invite).await
        }
        Err(error) if error.code() == ErrorCode::ConcurrentModification => {
            show_course_invite(screen, course_id).await
        }
        Err(error) => Err(error.into()),
    }
}
