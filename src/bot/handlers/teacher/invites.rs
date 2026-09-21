use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::app::{ErrorCode, GroupInviteState};
use crate::bot::HandlerResult;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, FlowState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{confirm_invite_rotation_keyboard, group_invite_keyboard};
use crate::domain::TeachingCourseId;

pub(in crate::bot::handlers) async fn show_group_invite(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let invite = screen
        .deps
        .context
        .services
        .courses
        .get_group_invite(screen.actor, course_id)
        .await?;

    send_group_invite(screen, course_id, &invite).await
}

async fn send_group_invite(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
    invite: &GroupInviteState,
) -> HandlerResult {
    screen
        .bot
        .send_message(
            screen.chat_id,
            format!(
                "🔗 Ссылка для регистрации в группу {}:",
                invite.group.code().as_str()
            ),
        )
        .await?;
    screen
        .bot
        .send_message(
            screen.chat_id,
            screen
                .deps
                .context
                .invites
                .link(invite.group.id(), invite.generation),
        )
        .await?;
    let sent = screen
        .bot
        .send_message(screen.chat_id, texts::INVITE_REUSE_HINT)
        .reply_markup(group_invite_keyboard(course_id))
        .await?;

    screen
        .deps
        .dialogue
        .update(BotState::Screen(ScreenState::GroupInvite {
            message_id: sent.id,
            course_id,
            generation: invite.generation,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn ask_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, generation)) = state.group_invite(message_id) else {
        return screen.stale().await;
    };
    let sent = screen
        .bot
        .send_message(screen.chat_id, texts::INVITE_ROTATION_CONFIRM)
        .reply_markup(confirm_invite_rotation_keyboard())
        .await?;
    screen
        .deps
        .dialogue
        .update(BotState::Flow(FlowState::ConfirmingInviteRotation {
            message_id: sent.id,
            course_id,
            generation,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn cancel_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, _)) = state.confirming_invite_rotation(message_id) else {
        return screen.stale().await;
    };
    screen
        .bot
        .edit_message_reply_markup(screen.chat_id, message_id)
        .await?;
    show_group_invite(screen, course_id).await
}

pub(in crate::bot::handlers) async fn confirm_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    state: &BotState,
) -> HandlerResult {
    let Some((course_id, generation)) = state.confirming_invite_rotation(message_id) else {
        return screen.stale().await;
    };

    screen
        .bot
        .edit_message_reply_markup(screen.chat_id, message_id)
        .await?;

    match screen
        .deps
        .context
        .services
        .courses
        .rotate_group_invite(screen.actor, course_id, generation)
        .await
    {
        Ok(invite) => {
            screen
                .bot
                .send_message(screen.chat_id, "♻️ Прежняя ссылка больше не работает.")
                .await?;
            send_group_invite(screen, course_id, &invite).await
        }
        Err(error) if error.code() == ErrorCode::ConcurrentModification => {
            show_group_invite(screen, course_id).await
        }
        Err(error) => Err(error.into()),
    }
}
