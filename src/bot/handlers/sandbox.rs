use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::MessageId;

use super::input::require_text;
use super::reset_dialogue;
use crate::app::ReviewSandbox;
use crate::bot::HandlerResult;
use crate::bot::context::{Screen, TextStep};
use crate::bot::state::{BotState, FlowState, ScreenState};
use crate::bot::texts;
use crate::bot::views::{
    list_text, sandbox_keyboard, send_message_chunks, teacher_keyboard, teacher_menu,
};
use crate::domain::ReviewDecision;
use crate::pagination::PageRequest;

pub(super) async fn show_page(screen: Screen<'_>, page: PageRequest) -> HandlerResult {
    let shown_at = Utc::now();
    let actor = screen.actor();
    let bot = screen.bot();
    let chat_id = screen.chat_id();
    let dialogue = screen.dialogue();
    let services = screen.services();
    let timezone = screen.timezone();
    let submissions = services.reviews.sandbox_page(actor, page).await?;
    if submissions.items.is_empty() && page.page == 0 {
        reset_dialogue(dialogue).await?;
        bot.send_message(chat_id, texts::SANDBOX_EMPTY)
            .reply_markup(teacher_menu(None))
            .await?;
        return Ok(());
    }

    let total = services.reviews.sandbox_size(actor, None).await?;
    let entries = submissions.items.iter().map(|submission| {
        texts::sandbox_entry(
            &submission.student_name,
            &submission.assignment_title,
            &timezone.format_short(submission.submitted_at),
            submission.kind,
        )
    });
    let text = list_text(&texts::sandbox_header(total), entries);
    send_message_chunks(bot, chat_id, text, None).await?;
    let message = bot
        .send_message(chat_id, texts::SANDBOX_CHOOSE)
        .reply_markup(sandbox_keyboard(
            &submissions.items,
            submissions.page,
            submissions.next_page,
        ))
        .await?;
    dialogue
        .update(BotState::Screen(ScreenState::SandboxSubmissions {
            message_id: message.id,
            shown_at,
        }))
        .await?;
    Ok(())
}

pub(super) async fn ask_comment(
    screen: Screen<'_>,
    message_id: MessageId,
    decision: ReviewDecision,
    state: &BotState,
) -> HandlerResult {
    let Some(cutoff) = state.sandbox_screen(message_id) else {
        return screen.stale().await;
    };
    let pending = screen
        .services()
        .reviews
        .sandbox_size(screen.actor(), Some(cutoff))
        .await?;
    if pending == 0 {
        screen
            .bot()
            .send_message(screen.chat_id(), texts::SANDBOX_NOTHING_TO_DECIDE)
            .await?;
        return Ok(());
    }

    screen
        .dialogue()
        .update(BotState::Flow(FlowState::AwaitingBulkReviewComment {
            decision,
            cutoff,
        }))
        .await?;
    screen
        .ask(&texts::bulk_review_prompt(decision, pending))
        .await
}

pub(super) async fn submit_comment(
    step: TextStep<'_>,
    decision: ReviewDecision,
    cutoff: DateTime<Utc>,
) -> HandlerResult {
    let bot = step.bot();
    let chat_id = step.chat_id();

    let Some(text) = require_text(bot, step.message()).await? else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let reviewed = step
        .services()
        .reviews
        .review_sandbox(ReviewSandbox {
            actor: screen.actor(),
            decision,
            cutoff,
            comment: text.to_owned(),
            reviewed_at: Utc::now(),
        })
        .await?;

    reset_dialogue(step.dialogue()).await?;
    bot.send_message(chat_id, texts::bulk_review_saved(decision, reviewed))
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}
