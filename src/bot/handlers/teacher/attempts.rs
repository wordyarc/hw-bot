use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::form::detach;
use super::gradebook::show_student_grades_page;
use crate::app::GrantExtraAttempts;
use crate::bot::HandlerResult;
use crate::bot::callbacks::{AmountChoice, FlowAction, PageTarget};
use crate::bot::context::{Screen, TextStep};
use crate::bot::handlers::input::require_attempts;
use crate::bot::handlers::reset_dialogue;
use crate::bot::state::{BotState, FlowState, StudentChoice};
use crate::bot::texts;
use crate::bot::views::{
    assignment_students_keyboard, attempts_keyboard, list_text, paginated_keyboard,
    send_message_chunks, teacher_keyboard,
};
use crate::domain::{AssignmentId, MAX_ATTEMPTS, UserId};
use crate::pagination::PageRequest;

pub(crate) async fn show_students_page(
    screen: Screen<'_>,
    assignment_id: AssignmentId,
    page: PageRequest,
) -> HandlerResult {
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let assignment = services
        .assignments
        .find_for_teacher(actor, assignment_id)
        .await?;
    let students = services
        .assignments
        .list_students_for_assignment(actor, assignment_id, page)
        .await?;
    let with_groups = assignment.groups().len() > 1;

    if students.items.is_empty() {
        bot.send_message(chat_id, texts::NO_STUDENTS).await?;
        if students.page == 0 {
            return Ok(());
        }
    } else {
        let entries = students.items.iter().map(|student| {
            let history = student.history;
            let mut marks = String::new();
            if history.has_pending_submission() {
                marks.push_str(texts::STUDENT_PENDING_MARK);
            }
            if history.has_accepted_submission() {
                marks.push_str(texts::STUDENT_ACCEPTED_MARK);
            }
            let name = student.student.display_name().as_str();
            let name = if with_groups {
                texts::group_student(&student.group_code, name)
            } else {
                name.to_owned()
            };
            texts::student_entry(
                &name,
                history.used_attempts(),
                history.extra_attempts(),
                &marks,
            )
        });
        let text = list_text(texts::STUDENTS_HEADER, entries);
        send_message_chunks(bot, chat_id, text, None).await?;
    }

    let message = bot
        .send_message(chat_id, texts::CHOOSE_STUDENT_PROMPT)
        .reply_markup(paginated_keyboard(
            assignment_students_keyboard(assignment_id, &students.items, with_groups),
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

pub(crate) async fn select_student(
    screen: Screen<'_>,
    message_id: MessageId,
    student_id: UserId,
    state: BotState,
) -> HandlerResult {
    let Some((purpose, list)) = state.choosing_student(message_id, student_id) else {
        return screen.stale().await;
    };
    let assignment_id = match purpose {
        StudentChoice::Grades { course_id } => {
            return show_student_grades_page(
                screen,
                course_id,
                student_id,
                PageRequest::default(),
                Some(list),
            )
            .await;
        }
        StudentChoice::ExtraAttempts { assignment_id } => assignment_id,
    };

    let sent = screen
        .bot()
        .send_message(screen.chat_id(), texts::EXTRA_ATTEMPTS_QUESTION)
        .reply_markup(attempts_keyboard(FlowAction::ExtraAttempts))
        .await?;
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingAttemptAmount {
            message_id: sent.id,
            assignment_id,
            student_id,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn handle_extra_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((assignment_id, student_id)) = state.awaiting_attempt_amount(message_id) else {
        return screen.stale().await;
    };
    let Some(amount) = chosen_amount(choice) else {
        return screen.ask(&texts::attempts_prompt(MAX_ATTEMPTS)).await;
    };
    grant_attempts(screen, message_id, assignment_id, student_id, amount).await
}

pub(super) fn chosen_amount(choice: AmountChoice) -> Option<u16> {
    match choice {
        AmountChoice::Preset(value) if value > 0 && value <= MAX_ATTEMPTS => Some(value),
        AmountChoice::Preset(_) | AmountChoice::Manual => None,
    }
}

async fn grant_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    student_id: UserId,
    amount: u16,
) -> HandlerResult {
    let progress = screen
        .services()
        .assignments
        .grant_extra_attempts(GrantExtraAttempts {
            assignment_id,
            student_id,
            actor: screen.actor(),
            amount,
        })
        .await?;
    reset_dialogue(screen.dialogue()).await?;
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    screen
        .bot()
        .send_message(
            screen.chat_id(),
            texts::attempts_granted(amount, progress.extra_attempts()),
        )
        .reply_markup(teacher_keyboard())
        .await?;
    show_students_page(screen, assignment_id, PageRequest::default()).await?;
    Ok(())
}

pub(crate) async fn submit_attempt_amount(
    step: TextStep<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    student_id: UserId,
) -> HandlerResult {
    let Some(amount) = require_attempts(step.bot(), step.message()).await? else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    grant_attempts(screen, message_id, assignment_id, student_id, amount).await
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
