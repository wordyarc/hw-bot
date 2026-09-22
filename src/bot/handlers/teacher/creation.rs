use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::attempts::chosen_amount;
use crate::bot::HandlerResult;
use crate::bot::callbacks::{AmountChoice, GradingChoice, MAX_ATTEMPTS_PREFIX};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::handlers::input::{
    ensure_text_limit, parse_max_score, parse_positive_attempts, require_text,
};
use crate::bot::state::{
    AssignmentTerms, BotState, DraftAssignment, FlowState, TermsForm, TermsStep,
};
use crate::bot::texts;
use crate::bot::views::{attempts_keyboard, grading_keyboard};
use crate::domain::{ASSIGNMENT_TITLE_MAX_LEN, USER_TEXT_MAX_LEN};
use crate::domain::{Grading, MAX_ATTEMPTS, MAX_SCORE, TeachingCourseId};

pub(crate) async fn handle_max_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((course_id, title, body)) = state.awaiting_max_attempts(message_id) else {
        return screen.stale().await;
    };
    let Some(max_attempts) = chosen_amount(choice) else {
        return screen.ask(&texts::attempts_prompt(MAX_ATTEMPTS)).await;
    };
    ask_grading(
        screen.chat(),
        DraftAssignment::new(course_id, title, body, max_attempts),
    )
    .await
}

pub(super) async fn ask_grading(chat: Chat<'_>, draft: DraftAssignment) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let sent = bot
        .send_message(chat_id, crate::bot::texts::GRADING_QUESTION)
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
            screen
                .dialogue()
                .update(BotState::Flow(FlowState::AwaitingGradingText { draft }))
                .await?;
            return screen.ask(&texts::max_score_prompt(MAX_SCORE)).await;
        }
    };

    screen
        .bot()
        .edit_message_reply_markup(screen.chat_id(), message_id)
        .await?;
    apply_grading(screen.chat(), draft, grading).await
}

pub(super) async fn apply_grading(
    chat: Chat<'_>,
    draft: DraftAssignment,
    grading: Grading,
) -> HandlerResult {
    let draft = DraftAssignment { grading, ..draft };

    let step = if grading.is_scored() {
        TermsStep::Policy
    } else {
        TermsStep::Deadline
    };
    super::terms::render(
        chat,
        TermsForm::new(AssignmentTerms::creating(draft), step),
        None,
    )
    .await
}

pub(super) async fn ask_max_attempts(
    chat: Chat<'_>,
    course_id: TeachingCourseId,
    title: String,
    body: String,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let sent = bot
        .send_message(chat_id, crate::bot::texts::ATTEMPTS_QUESTION)
        .reply_markup(attempts_keyboard(MAX_ATTEMPTS_PREFIX))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            course_id,
            title,
            body,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn submit_title(step: TextStep<'_>, course_id: TeachingCourseId) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();
    let dialogue = step.dialogue();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::TITLE_REQUIRED).await?;
        return Ok(());
    }
    if !ensure_text_limit(
        bot,
        chat_id,
        text,
        ASSIGNMENT_TITLE_MAX_LEN,
        texts::FIELD_TITLE,
    )
    .await?
    {
        return Ok(());
    }
    dialogue
        .update(BotState::Flow(FlowState::AwaitingAssignmentBody {
            course_id,
            title: text.to_owned(),
        }))
        .await?;
    step.chat()
        .ask(crate::bot::texts::ASSIGNMENT_BODY_PROMPT)
        .await?;

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

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(chat_id, texts::BODY_REQUIRED).await?;
        return Ok(());
    }
    if !ensure_text_limit(bot, chat_id, text, USER_TEXT_MAX_LEN, texts::FIELD_BODY).await? {
        return Ok(());
    }
    ask_max_attempts(step.chat(), course_id, title, text.to_owned()).await?;

    Ok(())
}

pub(crate) async fn submit_max_attempts(
    step: TextStep<'_>,
    course_id: TeachingCourseId,
    title: String,
    body: String,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(max_attempts) = parse_positive_attempts(text) else {
        bot.send_message(chat_id, texts::number_error(MAX_ATTEMPTS))
            .await?;
        return Ok(());
    };
    ask_grading(
        step.chat(),
        DraftAssignment::new(course_id, title, body, max_attempts),
    )
    .await?;

    Ok(())
}

pub(crate) async fn submit_grading_text(
    step: TextStep<'_>,
    draft: DraftAssignment,
) -> HandlerResult {
    let bot = step.bot();
    let message = step.message();
    let chat_id = step.chat_id();

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(grading) = parse_max_score(text) else {
        bot.send_message(chat_id, texts::number_error(MAX_SCORE))
            .await?;
        return Ok(());
    };
    apply_grading(step.chat(), draft, grading).await?;

    Ok(())
}
