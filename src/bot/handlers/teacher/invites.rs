use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::MessageId;

use crate::app::{ErrorCode, GroupInviteState};
use crate::domain::TeachingCourseId;

use crate::bot::HandlerResult;
use crate::bot::callbacks::FlowAction;
use crate::bot::context::Screen;
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotState, ScreenState};
use crate::bot::views::group_invite_keyboard;
use crate::bot::views::send_stale_button;

pub(in crate::bot::handlers) async fn show_group_invite(
    screen: Screen<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let invite = match screen
        .services
        .courses
        .get_group_invite(screen.actor, course_id)
        .await
    {
        Ok(invite) => invite,
        Err(error) => return send_app_error(screen.bot, screen.chat_id, error).await,
    };

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
            screen.invites.link(invite.group.id(), invite.generation),
        )
        .await?;
    let sent = screen
        .bot
        .send_message(
            screen.chat_id,
            "Ссылка многоразовая: её можно отправить всей группе.",
        )
        .reply_markup(group_invite_keyboard(course_id))
        .await?;

    screen
        .dialogue
        .update(BotState::Screen(ScreenState::GroupInvite {
            message_id: sent.id,
            course_id,
            generation: invite.generation,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn handle_invite_rotation(
    screen: Screen<'_>,
    message_id: MessageId,
    action: FlowAction,
    state: BotState,
) -> HandlerResult {
    let Some((course_id, generation)) = state.confirming_invite_rotation(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    screen
        .bot
        .edit_message_reply_markup(screen.chat_id, message_id)
        .await?;

    if action == FlowAction::CancelInviteRotation {
        return show_group_invite(screen, course_id).await;
    }

    match screen
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
        Err(error) => send_app_error(screen.bot, screen.chat_id, error).await,
    }
}
