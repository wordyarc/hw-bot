use chrono::{DateTime, Utc};
use teloxide::types::{InlineKeyboardMarkup, MessageId};

use super::courses::show_course_assignments_page;
use super::form::{
    FormScreen, apply_date, cancel, choose_amount, discard, finish, form_error, input_manually,
    render, render_input_result, step_back, take_text,
};
use crate::app::{AppError, ErrorCode};
use crate::bot::HandlerResult;
use crate::bot::callbacks::{
    AmountChoice, MakeupChoice, MakeupFormAction, PageTarget, ReplayableAction,
};
use crate::bot::context::{Screen, TextStep};
use crate::bot::handlers::input::parse_step_number;
use crate::bot::makeup_form::{MakeupForm, MakeupInputError, MakeupStep};
use crate::bot::state::{BotState, FlowState};
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;
use crate::bot::views::makeup_form_view;
use crate::domain::{DomainError, MakeupTerms, TeachingCourseId};
use crate::pagination::PageRequest;

impl FormScreen for MakeupForm {
    fn view(
        &self,
        timezone: &BotTimezone,
        now: DateTime<Utc>,
    ) -> Option<(String, InlineKeyboardMarkup)> {
        makeup_form_view(self, timezone, now)
    }

    fn into_state(self) -> BotState {
        BotState::Flow(FlowState::ConfiguringMakeup(self))
    }

    fn abandoned_text(&self) -> &'static str {
        texts::MAKEUP_DISCARDED
    }

    fn input_message(error: MakeupInputError) -> Option<String> {
        match error {
            MakeupInputError::UnexpectedStep => None,
            MakeupInputError::Domain(error) => Some(form_error(&error)),
        }
    }

    fn cancelled_to(&self) -> Option<ReplayableAction> {
        Some(ReplayableAction::Page(
            PageTarget::Course(self.course_id),
            0,
        ))
    }
}

pub(crate) async fn open(screen: Screen<'_>, course_id: TeachingCourseId) -> HandlerResult {
    let state = screen
        .services()
        .courses
        .makeup_state(screen.actor(), course_id)
        .await?;
    let form = MakeupForm::new(course_id, state.saved, state.undecided, screen.now());
    render(screen.chat(), form, None).await
}

pub(crate) async fn handle_answer(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: MakeupChoice,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state
        .current_makeup_form(message_id)
        .filter(|form| matches!(form.step, MakeupStep::Scale(_)))
    else {
        return screen.stale().await;
    };
    let Some(grading) = form.current_scale().map(|scale| scale.grading) else {
        return screen.stale().await;
    };
    let Some(terms) = choice.terms(grading) else {
        return input_manually(screen.chat(), form).await;
    };
    let result = terms
        .map_err(MakeupInputError::Domain)
        .and_then(|terms| form.answer(terms));
    render_input_result(screen, form, result).await
}

pub(crate) async fn handle_attempts(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    let Some(form) = state.makeup_form(message_id, MakeupStep::Attempts) else {
        return screen.stale().await;
    };
    choose_amount(screen, form, choice, MakeupForm::apply_attempts).await
}

pub(crate) async fn handle_form(
    screen: Screen<'_>,
    message_id: MessageId,
    action: MakeupFormAction,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.current_makeup_form(message_id) else {
        return screen.stale().await;
    };
    let step = form.step;
    match action {
        MakeupFormAction::Back => step_back(screen, form).await,
        _ if step != MakeupStep::Summary => screen.stale().await,
        MakeupFormAction::Save => save(screen, form).await,
        MakeupFormAction::Cancel => cancel(screen, form).await,
        MakeupFormAction::Date | MakeupFormAction::Attempts | MakeupFormAction::Disable => {
            if !form.edit(action) {
                return screen.stale().await;
            }
            render(screen.chat(), form, None).await
        }
    }
}

pub(crate) async fn submit_text(step: TextStep<'_>, form: MakeupForm) -> HandlerResult {
    let Some((screen, mut form, text)) = take_text(step, form).await? else {
        return Ok(());
    };
    if form.step == MakeupStep::Date {
        return match screen.timezone().parse_deadline(&text) {
            Some(until) => apply_date(screen, form, until).await,
            None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
        };
    }
    let Some(value) = parse_step_number(&text) else {
        return render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await;
    };
    let scored_scale = form
        .current_scale()
        .map(|scale| scale.grading)
        .filter(|grading| grading.is_scored());
    let result = match scored_scale {
        Some(grading) => MakeupTerms::scored(value, grading)
            .map_err(MakeupInputError::Domain)
            .and_then(|terms| form.answer(terms)),
        None => form.apply_attempts(value),
    };
    render_input_result(screen, form, result).await
}

async fn save(screen: Screen<'_>, mut form: MakeupForm) -> HandlerResult {
    let now = screen.now();
    if form.notice_end(now) {
        return render(screen.chat(), form, Some(texts::MAKEUP_ENDED_BEFORE_SAVE)).await;
    }
    let Some(command) = form.command(screen.actor(), now) else {
        return discard(screen.chat(), form).await;
    };
    let notice = match screen.services().courses.configure_makeup(command).await {
        Ok(_) => texts::MAKEUP_SAVED,
        Err(error) if error.code() == ErrorCode::ConcurrentModification => texts::MAKEUP_CONFLICT,
        Err(AppError::Domain(
            error @ (DomainError::MakeupUntilOutOfRange { .. }
            | DomainError::InvalidMakeupAttempts { .. }),
        )) => {
            return render(screen.chat(), form, Some(&form_error(&error))).await;
        }
        Err(error) => return Err(error.into()),
    };
    finish(screen, form.message_id, notice).await?;
    show_course_assignments_page(screen, form.course_id, PageRequest::default()).await
}
