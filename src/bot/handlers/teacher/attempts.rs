use crate::bot::context::{Deps, Screen, TextStep};
use crate::bot::handlers::input::{parse_positive_attempts, require_text};
use crate::bot::handlers::resolve_actor;
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::app::GrantExtraAttempts;
use crate::bot::callbacks::PageTarget;
use crate::bot::views::paginated_keyboard;
use crate::domain::{AssignmentId, MAX_ATTEMPTS, UserId};
use crate::pagination::PageRequest;

use crate::bot::HandlerResult;
use crate::bot::callbacks::{AmountChoice, EXTRA_ATTEMPTS_PREFIX};
use crate::bot::state::{BotState, FlowState, StudentChoice};
use crate::bot::views::{
    ask_input, assignment_students_keyboard, attempts_keyboard, send_message_chunks,
    send_stale_button, teacher_keyboard,
};

use super::gradebook::show_student_grades_page;

pub(in crate::bot::handlers) async fn show_students_page(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    page: PageRequest,
) -> HandlerResult {
    let Screen {
        bot,
        chat_id,
        actor,
        services,
        dialogue,
        ..
    } = screen;
    let students = services
        .assignments
        .list_students_for_assignment(actor, assignment_id, page)
        .await?;

    if students.items.is_empty() {
        bot.send_message(chat_id, "В группе пока нет студентов.")
            .await?;
    } else {
        let mut text = String::from("👥 Студенты группы:\n");
        for student in &students.items {
            let pending = if student.history.has_pending_submission() {
                " · ⏳ сдача на проверке"
            } else {
                ""
            };
            let accepted = if student.history.has_accepted_submission() {
                " · ✅ работа принята"
            } else {
                ""
            };
            text.push_str(&format!(
                "\n{} · использовано {} · дополнительных {}{}{}\n",
                student.student.display_name().as_str(),
                student.history.used_attempts(),
                student.history.extra_attempts(),
                pending,
                accepted,
            ));
        }
        send_message_chunks(bot, chat_id, text, None).await?;
    }

    let message = bot
        .send_message(chat_id, "Выберите студента:")
        .reply_markup(paginated_keyboard(
            assignment_students_keyboard(assignment_id, &students.items),
            PageTarget::Students(assignment_id),
            students.page,
            students.next_page,
        ))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingStudent {
            message_id: message.id,
            purpose: StudentChoice::ExtraAttempts { assignment_id },
            student_ids: students
                .items
                .iter()
                .map(|item| item.student.id())
                .collect(),
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn select_student(
    screen: Screen<'_>,
    message_id: MessageId,
    student_id: UserId,
    state: BotState,
) -> HandlerResult {
    let Some(purpose) = state.choosing_student(message_id, student_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    let assignment_id = match purpose {
        StudentChoice::Grades { course_id } => {
            return show_student_grades_page(screen, course_id, student_id, PageRequest::default())
                .await;
        }
        StudentChoice::ExtraAttempts { assignment_id } => assignment_id,
    };

    let sent = screen
        .bot
        .send_message(screen.chat_id, "Сколько дополнительных попыток выдать?")
        .reply_markup(attempts_keyboard(EXTRA_ATTEMPTS_PREFIX))
        .await?;
    screen
        .dialogue
        .update(BotState::Flow(FlowState::AwaitingAttemptAmount {
            message_id: sent.id,
            assignment_id,
            student_id,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn handle_extra_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((assignment_id, student_id)) = state.awaiting_attempt_amount(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    let Some(amount) = chosen_amount(choice) else {
        return ask_input(
            screen.bot,
            screen.chat_id,
            &format!("Введите целое число от 1 до {MAX_ATTEMPTS}:"),
        )
        .await;
    };
    grant_attempts(screen, assignment_id, student_id, amount).await
}

pub(super) fn chosen_amount(choice: AmountChoice) -> Option<u16> {
    match choice {
        AmountChoice::Preset(value) if value > 0 && value <= MAX_ATTEMPTS => Some(value),
        AmountChoice::Preset(_) | AmountChoice::Manual => None,
    }
}

pub(super) async fn grant_attempts(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    student_id: UserId,
    amount: u16,
) -> HandlerResult {
    let progress = screen
        .services
        .assignments
        .grant_extra_attempts(GrantExtraAttempts {
            assignment_id,
            student_id,
            actor: screen.actor,
            amount,
        })
        .await?;
    crate::bot::handlers::reset_dialogue(screen.dialogue).await?;
    screen
        .bot
        .send_message(
            screen.chat_id,
            format!(
                "🎁 Добавлено попыток: {amount}. Всего дополнительных: {}.",
                progress.extra_attempts()
            ),
        )
        .reply_markup(teacher_keyboard())
        .await?;
    show_students_page(screen, assignment_id, PageRequest::default()).await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn submit_attempt_amount(
    step: TextStep<'_>,
    assignment_id: AssignmentId,
    student_id: UserId,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id,
        deps,
    } = step;
    let Deps {
        services,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(amount) = parse_positive_attempts(text) else {
        bot.send_message(
            message.chat.id,
            format!("Введите целое число от 1 до {MAX_ATTEMPTS}."),
        )
        .await?;
        return Ok(());
    };
    let Some(actor) =
        resolve_actor(bot, message.chat.id, telegram_user_id, services, dialogue).await?
    else {
        return Ok(());
    };
    grant_attempts(
        Screen::new(bot, message.chat.id, actor, deps),
        assignment_id,
        student_id,
        amount,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_preset_within_the_attempt_limit_is_accepted() {
        assert_eq!(chosen_amount(AmountChoice::Preset(1)), Some(1));
        assert_eq!(
            chosen_amount(AmountChoice::Preset(MAX_ATTEMPTS)),
            Some(MAX_ATTEMPTS)
        );
    }

    #[test]
    fn a_preset_outside_the_attempt_limit_falls_back_to_manual_input() {
        assert_eq!(chosen_amount(AmountChoice::Preset(0)), None);
        assert_eq!(chosen_amount(AmountChoice::Preset(MAX_ATTEMPTS + 1)), None);
        assert_eq!(chosen_amount(AmountChoice::Manual), None);
    }
}
