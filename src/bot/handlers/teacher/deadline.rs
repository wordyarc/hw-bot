use crate::bot::context::{Deps, Screen, TextStep};
use crate::bot::handlers::input::{require_text, send_deadline_format_error};
use crate::bot::handlers::resolve_actor;
use chrono::{DateTime, Utc};
use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::types::MessageId;

use crate::app::{ChangeDeadline, CreateAssignment};

use crate::bot::HandlerResult;
use crate::bot::assignment_files;
use crate::bot::callbacks::DeadlineAction;
use crate::bot::errors::send_app_error;
use crate::bot::state::{BotDialogue, BotState, DeadlineTarget, FlowState};
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{
    ask_input, calendar_keyboard, deadline_options_keyboard, send_stale_button, teacher_keyboard,
};

use super::courses::show_assignment;

pub(in crate::bot::handlers) async fn handle_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    action: DeadlineAction,
    state: BotState,
) -> HandlerResult {
    let Some(target) = state.choosing_deadline(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    let date = match action {
        DeadlineAction::Days(days) => screen.timezone.in_days(u64::from(days)),
        DeadlineAction::Months(months) => screen.timezone.in_months(months),
        DeadlineAction::Pick(date) => Some(date),
        DeadlineAction::Manual => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingDeadlineText { target }))
                .await?;
            return ask_input(
                screen.bot,
                screen.chat_id,
                &format!(
                    "Введите дату в формате ДД.ММ.ГГГГ или ДД.ММ.ГГГГ ЧЧ:ММ ({}):",
                    screen.timezone.suffix()
                ),
            )
            .await;
        }
        DeadlineAction::Options => {
            return ask_deadline(
                screen.bot,
                screen.chat_id,
                target,
                screen.dialogue,
                screen.timezone,
            )
            .await;
        }
        DeadlineAction::Calendar => {
            let month = screen.timezone.today();
            screen
                .bot
                .edit_message_reply_markup(screen.chat_id, message_id)
                .reply_markup(calendar_keyboard(month, month))
                .await?;
            return Ok(());
        }
        DeadlineAction::Month(month) => {
            screen
                .bot
                .edit_message_reply_markup(screen.chat_id, message_id)
                .reply_markup(calendar_keyboard(month, screen.timezone.today()))
                .await?;
            return Ok(());
        }
    };

    let Some(deadline_at) = date.and_then(|date| screen.timezone.end_of(date)) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    apply_deadline(screen, target, deadline_at).await
}

fn deadline_prompt(target: &DeadlineTarget, timezone: &BotTimezone) -> String {
    let question = match target {
        DeadlineTarget::Create(_) => "5/5. Когда истекает дедлайн?",
        DeadlineTarget::Change { .. } => "Новый дедлайн.",
    };
    format!(
        "{question} Время по умолчанию — {} ({}).",
        timezone.deadline_time().format("%H:%M"),
        timezone.suffix(),
    )
}

pub(super) async fn ask_deadline(
    bot: &crate::bot::Bot,
    chat_id: teloxide::types::ChatId,
    target: DeadlineTarget,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, deadline_prompt(&target, timezone))
        .reply_markup(deadline_options_keyboard(timezone))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingDeadline {
            message_id: sent.id,
            target,
        }))
        .await?;
    Ok(())
}

pub(super) async fn apply_deadline(
    screen: Screen<'_>,
    target: DeadlineTarget,
    deadline_at: DateTime<Utc>,
) -> HandlerResult {
    match target {
        DeadlineTarget::Create(draft) => {
            let result = screen
                .services
                .assignments
                .create(CreateAssignment {
                    teaching_course_id: draft.course_id,
                    actor: screen.actor,
                    title: draft.title,
                    body: draft.body,
                    max_attempts: draft.max_attempts,
                    grading: draft.grading,
                    late_policy: draft.late_policy,
                    created_at: Utc::now(),
                    deadline_at,
                })
                .await;
            match result {
                Ok(assignment) => {
                    crate::bot::handlers::reset_dialogue(screen.dialogue).await?;
                    screen
                        .bot
                        .send_message(
                            screen.chat_id,
                            format!(
                                "📝 Черновик создан. Дедлайн: {}",
                                screen.timezone.format(deadline_at)
                            ),
                        )
                        .reply_markup(teacher_keyboard())
                        .await?;
                    assignment_files::show(screen, assignment.id(), true).await?;
                }
                Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
            }
        }
        DeadlineTarget::Change { assignment_id } => match screen
            .services
            .assignments
            .change_deadline(ChangeDeadline {
                assignment_id,
                actor: screen.actor,
                deadline_at,
            })
            .await
        {
            Ok(_) => {
                screen
                    .bot
                    .send_message(
                        screen.chat_id,
                        format!(
                            "⏰ Дедлайн изменён: {}",
                            screen.timezone.format(deadline_at)
                        ),
                    )
                    .reply_markup(teacher_keyboard())
                    .await?;
                show_assignment(screen, assignment_id).await?;
            }
            Err(error) => send_app_error(screen.bot, screen.chat_id, error).await?,
        },
    }
    Ok(())
}

pub(in crate::bot::handlers) async fn submit_deadline_text(
    step: TextStep<'_>,
    target: DeadlineTarget,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id,
        deps,
    } = step;
    let Deps {
        services,
        dialogue: _,
        timezone,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(deadline_at) = timezone.parse_deadline(text) else {
        send_deadline_format_error(bot, message.chat.id, timezone).await?;
        return Ok(());
    };
    let Some(actor) = resolve_actor(bot, message.chat.id, telegram_user_id, services).await? else {
        return Ok(());
    };
    apply_deadline(
        Screen::new(bot, message.chat.id, actor, deps),
        target,
        deadline_at,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, NaiveTime};

    use super::*;
    use crate::bot::state::DraftAssignment;
    use crate::domain::{AssignmentId, TeachingCourseId};

    fn timezone() -> BotTimezone {
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
    }

    #[test]
    fn the_deadline_prompt_is_numbered_only_while_creating() {
        let draft = DraftAssignment::new(
            TeachingCourseId::new(),
            "Rust".to_owned(),
            "Body".to_owned(),
            2,
        );

        let creating = deadline_prompt(&DeadlineTarget::Create(draft), &timezone());
        let changing = deadline_prompt(
            &DeadlineTarget::Change {
                assignment_id: AssignmentId::new(),
            },
            &timezone(),
        );

        assert!(creating.starts_with("5/5."));
        assert!(!changing.contains("5/5"));
    }

    #[test]
    fn the_deadline_prompt_names_the_default_time_and_the_offset() {
        let prompt = deadline_prompt(
            &DeadlineTarget::Change {
                assignment_id: AssignmentId::new(),
            },
            &timezone(),
        );

        assert!(prompt.contains("23:59"), "{prompt}");
        assert!(prompt.contains("UTC+10"), "{prompt}");
    }
}
