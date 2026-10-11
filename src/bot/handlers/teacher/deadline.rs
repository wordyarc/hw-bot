use chrono::{DateTime, NaiveDate, Utc};
use teloxide::types::MessageId;

use super::makeup::handle_date;
use super::terms::{apply_date, render};
use crate::bot::assignment_form::{InputMode, TermsStep};
use crate::bot::callbacks::DeadlineAction;
use crate::bot::context::Screen;
use crate::bot::makeup_form::MakeupStep;
use crate::bot::state::BotState;
use crate::bot::timezone::BotTimezone;
use crate::bot::{HandlerResult, texts};

pub(super) enum DateChoice {
    Mode(InputMode),
    At(Option<DateTime<Utc>>),
}

pub(super) fn choose_date(
    action: DeadlineAction,
    timezone: &BotTimezone,
    calendar_from: NaiveDate,
) -> DateChoice {
    let date = match action {
        DeadlineAction::Manual => return DateChoice::Mode(InputMode::Manual),
        DeadlineAction::Options => return DateChoice::Mode(InputMode::Buttons),
        DeadlineAction::Calendar => return DateChoice::Mode(InputMode::Calendar(calendar_from)),
        DeadlineAction::Month(month) => return DateChoice::Mode(InputMode::Calendar(month)),
        DeadlineAction::Days(days) => timezone.in_days(u64::from(days)),
        DeadlineAction::Months(months) => timezone.in_months(months),
        DeadlineAction::Pick(date) => Some(date),
    };
    DateChoice::At(date.and_then(|date| timezone.end_of(date)))
}

pub(crate) async fn handle_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    action: DeadlineAction,
    state: BotState,
) -> HandlerResult {
    if state.active_makeup().is_some() {
        return match state.makeup_form(message_id, MakeupStep::Date) {
            Some(form) => handle_date(screen, form, action).await,
            None => screen.stale().await,
        };
    }
    let Some(active) = state.active_terms() else {
        return screen.stale().await;
    };
    let step = active.step;
    let presets_allowed = step == TermsStep::Deadline
        || !matches!(action, DeadlineAction::Days(_) | DeadlineAction::Months(_));
    if !matches!(step, TermsStep::Deadline | TermsStep::DurationDate) || !presets_allowed {
        return screen.stale().await;
    }
    let Some(mut form) = state.terms_form(message_id, step) else {
        return screen.stale().await;
    };
    let timezone = screen.timezone();
    let calendar_from = match (step, form.terms.deadline_at) {
        (TermsStep::DurationDate, Some(deadline)) => timezone.first_date_ending_after(deadline),
        _ => timezone.today(),
    };
    match choose_date(action, timezone, calendar_from) {
        DateChoice::Mode(mode) => {
            form.mode = mode;
            render(screen.chat(), form, None).await
        }
        DateChoice::At(Some(at)) => apply_date(screen, form, at).await,
        DateChoice::At(None) => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
    }
}
