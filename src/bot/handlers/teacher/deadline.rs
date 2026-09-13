use teloxide::types::MessageId;

use crate::bot::callbacks::DeadlineAction;
use crate::bot::context::Screen;
use crate::bot::state::{BotState, TermsStep};
use crate::bot::{HandlerResult, texts};

use super::terms::{apply_date, render};

pub(in crate::bot::handlers) async fn handle_deadline(
    screen: Screen<'_>,
    message_id: MessageId,
    action: DeadlineAction,
    state: BotState,
) -> HandlerResult {
    let Some(active) = state.active_terms() else {
        return screen.stale().await;
    };
    let step = active.step;
    if !matches!(step, TermsStep::Deadline | TermsStep::DurationDate) {
        return screen.stale().await;
    }
    let Some(mut form) = state.terms_form(message_id, step) else {
        return screen.stale().await;
    };
    let date = match action {
        DeadlineAction::Manual => {
            form.manual = true;
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Options => {
            form.calendar_month = None;
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Calendar => {
            form.calendar_month = Some(if step == TermsStep::DurationDate {
                screen
                    .context
                    .timezone
                    .first_date_after(form.terms.deadline_at.expect("duration follows deadline"))
                    .unwrap_or(chrono::NaiveDate::MAX)
            } else {
                screen.context.timezone.today()
            });
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Month(month) => {
            form.calendar_month = Some(month);
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Days(days) if step == TermsStep::Deadline => {
            screen.context.timezone.in_days(u64::from(days))
        }
        DeadlineAction::Months(months) if step == TermsStep::Deadline => {
            screen.context.timezone.in_months(months)
        }
        DeadlineAction::Days(_) | DeadlineAction::Months(_) => {
            return screen.stale().await;
        }
        DeadlineAction::Pick(date) => Some(date),
    };
    match date.and_then(|date| screen.context.timezone.end_of(date)) {
        Some(at) => apply_date(screen, form, at).await,
        None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
    }
}
