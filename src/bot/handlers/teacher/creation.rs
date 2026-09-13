use crate::bot::context::{Deps, Screen, TextStep};
use crate::bot::handlers::input::{
    ensure_text_limit, parse_max_score, parse_positive_attempts, require_text,
};
use crate::domain::{ASSIGNMENT_TITLE_MAX_LEN, USER_TEXT_MAX_LEN};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use crate::domain::{Grading, MAX_ATTEMPTS, MAX_SCORE, TeachingCourseId};

use crate::bot::HandlerResult;
use crate::bot::callbacks::{AmountChoice, GradingChoice, MAX_ATTEMPTS_PREFIX};
use crate::bot::state::{
    AssignmentTerms, BotDialogue, BotState, DraftAssignment, FlowState, TermsForm, TermsStep,
};
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{ask_input, attempts_keyboard, grading_keyboard, send_stale_button};

use super::attempts::chosen_amount;

pub(in crate::bot::handlers) async fn handle_max_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some((course_id, title, body)) = state.awaiting_max_attempts(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };
    let Some(max_attempts) = chosen_amount(choice) else {
        return ask_input(
            screen.bot,
            screen.chat_id,
            &format!("Введите количество попыток от 1 до {MAX_ATTEMPTS}:"),
        )
        .await;
    };
    ask_grading(
        screen.bot,
        screen.chat_id,
        DraftAssignment::new(course_id, title, body, max_attempts),
        screen.dialogue,
    )
    .await
}

pub(super) async fn ask_grading(
    bot: &crate::bot::Bot,
    chat_id: teloxide::types::ChatId,
    draft: DraftAssignment,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, crate::bot::texts::GRADING_QUESTION)
        .reply_markup(grading_keyboard())
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingGrading {
            message_id: sent.id,
            draft,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn handle_grading(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: GradingChoice,
    state: BotState,
) -> HandlerResult {
    let Some(draft) = state.choosing_grading(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    let grading = match choice {
        GradingChoice::PassFail => Grading::PassFail,
        GradingChoice::Preset(max_score) => match Grading::scored(max_score) {
            Ok(grading) => grading,
            Err(_) => return send_stale_button(screen.bot, screen.chat_id).await,
        },
        GradingChoice::Manual => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingGradingText { draft }))
                .await?;
            return ask_input(
                screen.bot,
                screen.chat_id,
                &format!("Введите максимальный балл от 1 до {MAX_SCORE}:"),
            )
            .await;
        }
    };

    screen
        .bot
        .edit_message_reply_markup(screen.chat_id, message_id)
        .await?;
    apply_grading(
        screen.bot,
        screen.chat_id,
        draft,
        grading,
        screen.dialogue,
        screen.timezone,
    )
    .await
}

pub(super) async fn apply_grading(
    bot: &crate::bot::Bot,
    chat_id: teloxide::types::ChatId,
    draft: DraftAssignment,
    grading: Grading,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    let draft = DraftAssignment { grading, ..draft };

    let step = if grading.is_scored() {
        TermsStep::Policy
    } else {
        TermsStep::Deadline
    };
    super::terms::render(
        bot,
        chat_id,
        dialogue,
        timezone,
        TermsForm::new(AssignmentTerms::creating(draft), step),
        None,
    )
    .await
}

pub(super) async fn ask_max_attempts(
    bot: &crate::bot::Bot,
    chat_id: teloxide::types::ChatId,
    course_id: TeachingCourseId,
    title: String,
    body: String,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, crate::bot::texts::ATTEMPTS_QUESTION)
        .reply_markup(attempts_keyboard(MAX_ATTEMPTS_PREFIX))
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
            message_id: sent.id,
            course_id,
            title,
            body,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn submit_title(
    step: TextStep<'_>,
    course_id: TeachingCourseId,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id: _,
        deps,
    } = step;
    let Deps {
        services: _,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(message.chat.id, "Название не должно быть пустым.")
            .await?;
        return Ok(());
    }
    if !ensure_text_limit(
        bot,
        message.chat.id,
        text,
        ASSIGNMENT_TITLE_MAX_LEN,
        "Название",
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
    ask_input(
        bot,
        message.chat.id,
        crate::bot::texts::ASSIGNMENT_BODY_PROMPT,
    )
    .await?;

    Ok(())
}

pub(in crate::bot::handlers) async fn submit_body(
    step: TextStep<'_>,
    course_id: TeachingCourseId,
    title: String,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id: _,
        deps,
    } = step;
    let Deps {
        services: _,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    if text.trim().is_empty() {
        bot.send_message(message.chat.id, "Текст задания не должен быть пустым.")
            .await?;
        return Ok(());
    }
    if !ensure_text_limit(
        bot,
        message.chat.id,
        text,
        USER_TEXT_MAX_LEN,
        "Текст задания",
    )
    .await?
    {
        return Ok(());
    }
    ask_max_attempts(
        bot,
        message.chat.id,
        course_id,
        title,
        text.to_owned(),
        dialogue,
    )
    .await?;

    Ok(())
}

pub(in crate::bot::handlers) async fn submit_max_attempts(
    step: TextStep<'_>,
    course_id: TeachingCourseId,
    title: String,
    body: String,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id: _,
        deps,
    } = step;
    let Deps {
        services: _,
        dialogue,
        timezone: _,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(max_attempts) = parse_positive_attempts(text) else {
        bot.send_message(
            message.chat.id,
            format!("Введите целое число от 1 до {MAX_ATTEMPTS}."),
        )
        .await?;
        return Ok(());
    };
    ask_grading(
        bot,
        message.chat.id,
        DraftAssignment::new(course_id, title, body, max_attempts),
        dialogue,
    )
    .await?;

    Ok(())
}

pub(in crate::bot::handlers) async fn submit_grading_text(
    step: TextStep<'_>,
    draft: DraftAssignment,
) -> HandlerResult {
    let TextStep {
        bot,
        message,
        telegram_user_id: _,
        deps,
    } = step;
    let Deps {
        services: _,
        dialogue,
        timezone,
        invites: _,
    } = deps;

    let Some(text) = require_text(bot, message).await? else {
        return Ok(());
    };
    let Some(grading) = parse_max_score(text) else {
        bot.send_message(
            message.chat.id,
            format!("Введите целое число от 1 до {MAX_SCORE}."),
        )
        .await?;
        return Ok(());
    };
    apply_grading(bot, message.chat.id, draft, grading, dialogue, timezone).await?;

    Ok(())
}
