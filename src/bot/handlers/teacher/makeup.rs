use chrono::{DateTime, Utc};
use teloxide::types::{InlineKeyboardMarkup, MessageId};

use super::courses::show_course_assignments_page;
use super::form::{
    FormScreen, apply_date, cancel, discard, finish, render, render_input_result, step_back,
    take_text,
};
use crate::app::{AppError, ErrorCode};
use crate::bot::HandlerResult;
use crate::bot::callbacks::{
    AmountChoice, MakeupChoice, MakeupFormAction, PageTarget, ReplayableAction,
};
use crate::bot::context::{Screen, TextStep};
use crate::bot::form::InputMode;
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
        &mut self,
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
            MakeupInputError::Domain(error) => Some(input_error(&error)),
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
    let form = MakeupForm::new(course_id, state.saved, state.undecided, Utc::now());
    render(screen.chat(), form, None).await
}

fn input_error(error: &DomainError) -> String {
    match error {
        DomainError::MakeupUntilOutOfRange { max_days } => {
            texts::makeup_until_out_of_range(*max_days)
        }
        DomainError::InvalidMakeupAttempts { max, .. }
        | DomainError::InvalidMakeupCredit { max, .. } => texts::number_prompt(*max),
        DomainError::MakeupTermsDoNotFitGrading => texts::CHOOSE_BUTTON.to_owned(),
        _ => texts::INVALID_TERMS.to_owned(),
    }
}

fn scale_step(state: &BotState) -> Option<MakeupStep> {
    state
        .active_makeup()
        .map(|form| form.step)
        .filter(|step| matches!(step, MakeupStep::Scale(_)))
}

pub(crate) async fn handle_answer(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: MakeupChoice,
    state: BotState,
) -> HandlerResult {
    let Some(step) = scale_step(&state) else {
        return screen.stale().await;
    };
    let Some(mut form) = state.makeup_form(message_id, step) else {
        return screen.stale().await;
    };
    let Some(grading) = form.current_scale().map(|scale| scale.grading) else {
        return screen.stale().await;
    };
    let terms = match choice {
        MakeupChoice::Manual => {
            form.mode = InputMode::Manual;
            return render(screen.chat(), form, None).await;
        }
        MakeupChoice::NotOffered => Ok(MakeupTerms::NotOffered),
        MakeupChoice::PassFail => Ok(MakeupTerms::PassFail),
        MakeupChoice::Credit(credit) => MakeupTerms::scored(credit, grading),
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
    let Some(mut form) = state.makeup_form(message_id, MakeupStep::Attempts) else {
        return screen.stale().await;
    };
    match choice {
        AmountChoice::Preset(attempts) => {
            let result = form.apply_attempts(attempts);
            render_input_result(screen, form, result).await
        }
        AmountChoice::Manual => {
            form.mode = InputMode::Manual;
            render(screen.chat(), form, None).await
        }
    }
}

pub(crate) async fn handle_form(
    screen: Screen<'_>,
    message_id: MessageId,
    action: MakeupFormAction,
    state: BotState,
) -> HandlerResult {
    let Some(step) = state.active_makeup().map(|form| form.step) else {
        return screen.stale().await;
    };
    let Some(mut form) = state.makeup_form(message_id, step) else {
        return screen.stale().await;
    };
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
    let now = Utc::now();
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
            return render(screen.chat(), form, Some(&input_error(&error))).await;
        }
        Err(error) => return Err(error.into()),
    };
    finish(screen, form.message_id, notice).await?;
    show_course_assignments_page(screen, form.course_id, PageRequest::default()).await
}
