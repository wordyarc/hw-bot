use chrono::{DateTime, NaiveDate, Utc};
use teloxide::types::MessageId;

use super::form::handle_date;
use crate::bot::HandlerResult;
use crate::bot::assignment_form::TermsStep;
use crate::bot::callbacks::DeadlineAction;
use crate::bot::context::Screen;
use crate::bot::form::InputMode;
use crate::bot::makeup_form::MakeupStep;
use crate::bot::state::BotState;
use crate::bot::timezone::BotTimezone;

pub(super) enum DateChoice {
    Mode(InputMode),
    At(Option<DateTime<Utc>>),
}

pub(super) fn choose_date(
    action: DeadlineAction,
    timezone: &BotTimezone,
    calendar_from: NaiveDate,
) -> DateChoice {
    match action {
        DeadlineAction::Manual => DateChoice::Mode(InputMode::Manual),
        DeadlineAction::Options => DateChoice::Mode(InputMode::Buttons),
        DeadlineAction::Calendar => DateChoice::Mode(InputMode::Calendar(calendar_from)),
        DeadlineAction::Month(month) => DateChoice::Mode(InputMode::Calendar(month)),
        DeadlineAction::Pick(date) => DateChoice::At(timezone.end_of(date)),
    }
}

pub(crate) async fn handle_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    action: DeadlineAction,
    state: BotState,
) -> HandlerResult {
    if state.active_makeup().is_some() {
        return match state.makeup_form(message_id, MakeupStep::Date) {
            Some(form) => {
                let today = screen.timezone().local_date(screen.now());
                handle_date(screen, form, action, today).await
            }
            None => screen.stale().await,
        };
    }
    let Some(form) = state.current_terms_form(message_id) else {
        return screen.stale().await;
    };
    if !matches!(form.step, TermsStep::Deadline | TermsStep::DurationDate) {
        return screen.stale().await;
    }
    let timezone = screen.timezone();
    let calendar_from = match (form.step, form.terms.deadline_at) {
        (TermsStep::DurationDate, Some(deadline)) => timezone.first_date_ending_after(deadline),
        _ => timezone.local_date(screen.now()),
    };
    handle_date(screen, form, action, calendar_from).await
}
