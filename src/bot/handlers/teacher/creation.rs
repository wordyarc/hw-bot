use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::attempts::chosen_amount;
use crate::bot::HandlerResult;
use crate::bot::assignment_form::{AssignmentTerms, AssignmentText, DraftAssignment, TermsForm};
use crate::bot::callbacks::{AmountChoice, GradingChoice, MAX_ATTEMPTS_PREFIX};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::handlers::input::{
    ensure_text_limit, parse_max_score, parse_positive_attempts, require_text,
};
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{attempts_keyboard, grading_keyboard};
use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, Grading, MAX_ATTEMPTS, MAX_SCORE, TeachingCourseId, USER_TEXT_MAX_LEN,
};

pub(crate) async fn handle_max_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some(text) = state.awaiting_max_attempts(message_id) else {
        return screen.stale().await;
    };
    let Some(max_attempts) = chosen_amount(choice) else {
        return screen.ask(&texts::attempts_prompt(MAX_ATTEMPTS)).await;
    };
    ask_grading(screen.chat(), DraftAssignment::new(text, max_attempts)).await
}

pub(super) async fn ask_grading(chat: Chat<'_>, draft: DraftAssignment) -> HandlerResult {
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
            screen
                .dialogue()
                .update(BotState::Flow(FlowState::AwaitingGradingText { draft }))
                .await?;
            return screen.ask(&texts::max_score_prompt(MAX_SCORE)).await;
        }
    };

    apply_grading(screen, draft, grading, Some(message_id)).await
}

pub(super) async fn apply_grading(
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
    super::terms::render(screen.chat(), form, None).await
}

pub(super) async fn ask_max_attempts(chat: Chat<'_>, text: AssignmentText) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    let sent = bot
        .send_message(chat_id, texts::ATTEMPTS_QUESTION)
        .reply_markup(attempts_keyboard(MAX_ATTEMPTS_PREFIX))
        .await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            text,
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
    let text = AssignmentText {
        course_id,
        title,
        body: text.to_owned(),
    };
    ask_max_attempts(step.chat(), text).await?;

    Ok(())
}

pub(crate) async fn submit_max_attempts(
    step: TextStep<'_>,
    assignment: AssignmentText,
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
    ask_grading(step.chat(), DraftAssignment::new(assignment, max_attempts)).await?;

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
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    apply_grading(screen, draft, grading, None).await?;

    Ok(())
}
