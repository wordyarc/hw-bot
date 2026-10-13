use std::collections::BTreeSet;

use teloxide::payloads::{EditMessageReplyMarkupSetters, SendMessageSetters};
use teloxide::types::MessageId;

use super::attempts::chosen_amount;
use super::form::{detach, render};
use crate::bot::HandlerResult;
use crate::bot::assignment_form::{
    AssignmentTerms, AssignmentText, DraftAssignment, IssuedGroups, TermsForm,
};
use crate::bot::callbacks::{AmountChoice, FlowAction, GradingChoice};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::handlers::input::{
    parse_max_score, require_attempts, require_text, require_text_within,
};
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{assignment_groups_keyboard, attempts_keyboard, grading_keyboard};
use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, Grading, MAX_ATTEMPTS, MAX_SCORE, StudentGroup, StudentGroupId,
    TeachingCourseId, USER_TEXT_MAX_LEN,
};

pub(crate) async fn handle_max_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((text, groups)) = state.awaiting_max_attempts(message_id) else {
        return screen.stale().await;
    };
    let Some(max_attempts) = chosen_amount(choice) else {
        return screen.ask(&texts::attempts_prompt(MAX_ATTEMPTS)).await;
    };
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    ask_grading(
        screen.chat(),
        DraftAssignment::new(text, groups, max_attempts),
    )
    .await
}

async fn ask_grading(chat: Chat<'_>, draft: DraftAssignment) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let sent = bot
        .send_message(chat_id, texts::GRADING_QUESTION)
        .reply_markup(grading_keyboard())
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingGrading {
            message_id: sent.id,
            draft,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn handle_grading(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: GradingChoice,
    state: BotState,
) -> HandlerResult {
    let Some(draft) = state.choosing_grading(message_id) else {
        return screen.stale().await;
    };

    let grading = match choice {
        GradingChoice::PassFail => Grading::PassFail,
        GradingChoice::Preset(max_score) => match Grading::scored(max_score) {
            Ok(grading) => grading,
            Err(_) => return screen.stale().await,
        },
        GradingChoice::Manual => {
            detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
            screen
                .dialogue()
                .update(BotState::Flow(FlowState::AwaitingGradingText { draft }))
                .await?;
            return screen.ask(&texts::max_score_prompt(MAX_SCORE)).await;
        }
    };

    apply_grading(screen, draft, grading, Some(message_id)).await
}

async fn apply_grading(
    screen: Screen<'_>,
    draft: DraftAssignment,
    grading: Grading,
    message_id: Option<MessageId>,
) -> HandlerResult {
    let draft = DraftAssignment { grading, ..draft };
    let question = screen
        .services()
        .courses
        .makeup_question(screen.actor(), draft.text.course_id, grading, None)
        .await?;

    let terms = AssignmentTerms::creating(draft, question);
    let step = terms.first_step();
    let mut form = TermsForm::new(terms, step);
    form.message_id = message_id;
    render(screen.chat(), form, None).await
}

async fn ask_max_attempts(
    chat: Chat<'_>,
    text: AssignmentText,
    groups: IssuedGroups,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let sent = bot
        .send_message(chat_id, texts::ATTEMPTS_QUESTION)
        .reply_markup(attempts_keyboard(FlowAction::MaxAttempts))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            text,
            groups,
        }))
        .await?;
    Ok(())
}

async fn ask_groups(
    chat: Chat<'_>,
    text: AssignmentText,
    groups: &[StudentGroup],
) -> HandlerResult {
    let selected: BTreeSet<StudentGroupId> = groups.iter().map(StudentGroup::id).collect();
    let sent = chat
        .bot()
        .send_message(chat.chat_id(), texts::ASSIGNMENT_GROUPS_QUESTION)
        .reply_markup(assignment_groups_keyboard(groups, &selected))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ChoosingAssignmentGroups {
            message_id: sent.id,
            text,
            selected,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn toggle_assignment_group(
    screen: Screen<'_>,
    message_id: MessageId,
    group_id: StudentGroupId,
    state: BotState,
) -> HandlerResult {
    let Some((text, mut selected)) = state.choosing_assignment_groups(message_id) else {
        return screen.stale().await;
    };
    let course = screen
        .services()
        .courses
        .find_for_teacher(screen.actor(), text.course_id)
        .await?;
    if !course.course.includes_group(group_id) {
        return screen.stale().await;
    }
    if !selected.remove(&group_id) {
        selected.insert(group_id);
    }
    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .reply_markup(assignment_groups_keyboard(&course.groups, &selected))
        .await?;
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::ChoosingAssignmentGroups {
            message_id,
            text,
            selected,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn finish_assignment_groups(
    screen: Screen<'_>,
    message_id: MessageId,
    state: BotState,
) -> HandlerResult {
    let Some((text, selected)) = state.choosing_assignment_groups(message_id) else {
        return screen.stale().await;
    };
    if selected.is_empty() {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::ASSIGNMENT_GROUPS_REQUIRED)
            .await?;
        return Ok(());
    }
    let course = screen
        .services()
        .courses
        .find_for_teacher(screen.actor(), text.course_id)
        .await?;
    let chosen = course
        .groups
        .into_iter()
        .filter(|group| selected.contains(&group.id()))
        .collect();
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    ask_max_attempts(screen.chat(), text, IssuedGroups::Chosen(chosen)).await
}

pub(crate) async fn submit_title(step: TextStep<'_>, course_id: TeachingCourseId) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let dialogue = step.dialogue();

    let Some(text) =
        require_text_within(bot, message, ASSIGNMENT_TITLE_MAX_LEN, texts::FIELD_TITLE).await?
    else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::TITLE_REQUIRED).await?;
        return Ok(());
    }
    dialogue
        .update(BotState::Flow(FlowState::AwaitingAssignmentBody {
            course_id,
            title: text.into_owned(),
        }))
        .await?;
    step.chat().ask(texts::ASSIGNMENT_BODY_PROMPT).await?;

    Ok(())
}

pub(crate) async fn submit_body(
    step: TextStep<'_>,
    course_id: TeachingCourseId,
    title: String,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();

    let Some(text) =
        require_text_within(bot, message, USER_TEXT_MAX_LEN, texts::FIELD_BODY).await?
    else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::BODY_REQUIRED).await?;
        return Ok(());
    }
    let text = AssignmentText {
        course_id,
        title,
        body: text.into_owned(),
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let course = screen
        .services()
        .courses
        .find_for_teacher(screen.actor(), course_id)
        .await?;
    match course.groups.as_slice() {
        [group] => ask_max_attempts(screen.chat(), text, IssuedGroups::Single(group.id())).await,
        groups => ask_groups(screen.chat(), text, groups).await,
    }
}

pub(crate) async fn submit_max_attempts(
    step: TextStep<'_>,
    message_id: MessageId,
    assignment: AssignmentText,
    groups: IssuedGroups,
) -> HandlerResult {
    let Some(max_attempts) = require_attempts(step.bot(), step.message()).await? else {
        return Ok(());
    };
    detach(step.bot(), step.chat_id(), Some(message_id)).await?;
    ask_grading(
        step.chat(),
        DraftAssignment::new(assignment, groups, max_attempts),
    )
    .await
}

pub(crate) async fn submit_grading_text(
    step: TextStep<'_>,
    message_id: Option<MessageId>,
    draft: DraftAssignment,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(grading) = parse_max_score(&text) else {
        bot.send_message(chat_id, texts::max_score_prompt(MAX_SCORE))
            .await?;
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    detach(bot, chat_id, message_id).await?;
    apply_grading(screen, draft, grading, None).await?;

    Ok(())
}
