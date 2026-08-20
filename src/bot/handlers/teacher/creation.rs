use crate::bot::handlers::{
    Deps, TextStep, ensure_text_limit, parse_late_policy, parse_max_score, parse_positive_attempts,
    require_text,
};
use crate::domain::{ASSIGNMENT_TITLE_MAX_LEN, USER_TEXT_MAX_LEN};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::MessageId;

use crate::domain::{Grading, LatePolicy, MAX_ATTEMPTS, MAX_SCORE, TeachingCourseId};

use crate::bot::HandlerResult;
use crate::bot::callbacks::{AmountChoice, GradingChoice, LatePolicyChoice, MAX_ATTEMPTS_PREFIX};
use crate::bot::handlers::Screen;
use crate::bot::state::{BotDialogue, BotState, DeadlineTarget, DraftAssignment, FlowState};
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{
    ask_input, attempts_keyboard, grading_keyboard, late_policy_keyboard, send_stale_button,
};

use super::attempts::chosen_amount;
use super::deadline::ask_deadline;

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
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    draft: DraftAssignment,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, "4/5. Как оцениваете задание?")
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
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    draft: DraftAssignment,
    grading: Grading,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    let draft = DraftAssignment { grading, ..draft };

    if !grading.is_scored() {
        return ask_deadline(
            bot,
            chat_id,
            DeadlineTarget::Create(draft),
            dialogue,
            timezone,
        )
        .await;
    }

    let sent = bot
        .send_message(chat_id, crate::bot::texts::LATE_POLICY_QUESTION)
        .reply_markup(late_policy_keyboard())
        .await?;
    dialogue
        .update(BotState::Flow(FlowState::ChoosingLatePolicy {
            message_id: sent.id,
            draft,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn handle_late_policy(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: LatePolicyChoice,
    state: BotState,
) -> HandlerResult {
    let Some(draft) = state.choosing_late_policy(message_id) else {
        return send_stale_button(screen.bot, screen.chat_id).await;
    };

    let late_policy = match choice {
        LatePolicyChoice::None => None,
        LatePolicyChoice::Preset {
            period_days,
            penalty,
            max_periods,
        } => match LatePolicy::new(period_days, penalty, max_periods) {
            Ok(policy) => Some(policy),
            Err(_) => return send_stale_button(screen.bot, screen.chat_id).await,
        },
        LatePolicyChoice::Manual => {
            screen
                .dialogue
                .update(BotState::Flow(FlowState::AwaitingLatePolicyText { draft }))
                .await?;
            return ask_input(
                screen.bot,
                screen.chat_id,
                "Введите три числа через пробел: шаг в днях, штраф за шаг и число шагов до \
                 закрытия приёма. Например «7 1 4»:",
            )
            .await;
        }
    };

    apply_late_policy(
        screen.bot,
        screen.chat_id,
        draft,
        late_policy,
        screen.dialogue,
        screen.timezone,
    )
    .await
}

pub(super) async fn apply_late_policy(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    draft: DraftAssignment,
    late_policy: Option<LatePolicy>,
    dialogue: &BotDialogue,
    timezone: &BotTimezone,
) -> HandlerResult {
    ask_deadline(
        bot,
        chat_id,
        DeadlineTarget::Create(DraftAssignment {
            late_policy,
            ..draft
        }),
        dialogue,
        timezone,
    )
    .await
}

pub(super) async fn ask_max_attempts(
    bot: &teloxide::Bot,
    chat_id: teloxide::types::ChatId,
    course_id: TeachingCourseId,
    title: String,
    body: String,
    dialogue: &BotDialogue,
) -> HandlerResult {
    let sent = bot
        .send_message(chat_id, "3/5. Сколько попыток даёте студенту?")
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
        "2/5. Пришлите текст задания одним сообщением:",
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

pub(in crate::bot::handlers) async fn submit_late_policy_text(
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
    let Some(late_policy) = parse_late_policy(text) else {
        bot.send_message(
            message.chat.id,
            "Введите три числа через пробел, например «7 1 4».",
        )
        .await?;
        return Ok(());
    };
    apply_late_policy(
        bot,
        message.chat.id,
        draft,
        Some(late_policy),
        dialogue,
        timezone,
    )
    .await?;

    Ok(())
}
