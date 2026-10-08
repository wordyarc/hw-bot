use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{ChatId, MessageId};

use super::courses::show_course_assignments_page;
use super::terms::{detach, show_step};
use crate::app::{AppError, ErrorCode};
use crate::bot::assignment_form::InputMode;
use crate::bot::callbacks::{AmountChoice, DeadlineAction, MakeupChoice, MakeupFormAction};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::handlers::input::{parse_number, require_text};
use crate::bot::handlers::reset_dialogue;
use crate::bot::makeup_form::{MakeupForm, MakeupInputError, MakeupStep};
use crate::bot::state::{BotState, FlowState};
use crate::bot::views::{makeup_form_view, teacher_keyboard};
use crate::bot::{Bot, HandlerResult, texts};
use crate::domain::{DomainError, MakeupTerms, TeachingCourseId};
use crate::pagination::PageRequest;

pub(crate) async fn open(screen: Screen<'_>, course_id: TeachingCourseId) -> HandlerResult {
    let state = screen
        .services()
        .courses
        .makeup_state(screen.actor(), course_id)
        .await?;
    let form = MakeupForm::new(course_id, state.saved, state.undecided, Utc::now());
    render(screen.chat(), form, None).await
}

async fn render(chat: Chat<'_>, mut form: MakeupForm, error: Option<&str>) -> HandlerResult {
    let Some((mut text, keyboard)) = makeup_form_view(&form, chat.timezone(), Utc::now()) else {
        return discard(chat, form).await;
    };
    if let Some(error) = error {
        text = format!("{error}\n\n{text}");
    }
    show_step(chat, &mut form.message_id, form.mode, text, keyboard).await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ConfiguringMakeup(form)))
        .await?;
    Ok(())
}

async fn discard(chat: Chat<'_>, form: MakeupForm) -> HandlerResult {
    tracing::error!(step = ?form.step, "makeup form does not hold what its step needs");
    reset_dialogue(chat.dialogue()).await?;
    abandon(chat.bot(), chat.chat_id(), &form).await
}

pub(crate) async fn abandon(bot: &Bot, chat_id: ChatId, form: &MakeupForm) -> HandlerResult {
    detach(bot, chat_id, form.message_id).await?;
    bot.send_message(chat_id, texts::MAKEUP_DISCARDED)
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}

fn input_error(error: &DomainError) -> String {
    match error {
        DomainError::MakeupUntilOutOfRange { max_days } => {
            texts::makeup_until_out_of_range(*max_days)
        }
        DomainError::InvalidMakeupAttempts { max, .. }
        | DomainError::InvalidMakeupCredit { max, .. } => texts::number_error(*max),
        DomainError::MakeupTermsDoNotFitGrading => texts::CHOOSE_BUTTON.to_owned(),
        _ => texts::INVALID_TERMS.to_owned(),
    }
}

async fn render_input_result(
    screen: Screen<'_>,
    form: MakeupForm,
    result: Result<(), MakeupInputError>,
) -> HandlerResult {
    let message = match result {
        Ok(()) => return render(screen.chat(), form, None).await,
        Err(MakeupInputError::UnexpectedStep) => return screen.stale().await,
        Err(MakeupInputError::Domain(error)) => input_error(&error),
    };
    render(screen.chat(), form, Some(&message)).await
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
    if action == MakeupFormAction::Back {
        if !form.can_go_back() {
            return screen.stale().await;
        }
        return back(screen, form).await;
    }
    if step != MakeupStep::Summary {
        return screen.stale().await;
    }
    match action {
        MakeupFormAction::Save => save(screen, form).await,
        MakeupFormAction::Cancel => cancel(screen, form).await,
        MakeupFormAction::Date
        | MakeupFormAction::Attempts
        | MakeupFormAction::Disable
        | MakeupFormAction::Back => {
            if !form.edit(action) {
                return screen.stale().await;
            }
            render(screen.chat(), form, None).await
        }
    }
}

pub(crate) async fn handle_date(
    screen: Screen<'_>,
    mut form: MakeupForm,
    action: DeadlineAction,
) -> HandlerResult {
    let timezone = screen.timezone();
    let date = match action {
        DeadlineAction::Manual => {
            form.mode = InputMode::Manual;
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Options => {
            form.mode = InputMode::Buttons;
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Calendar => {
            form.mode = InputMode::Calendar(timezone.today());
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Month(month) => {
            form.mode = InputMode::Calendar(month);
            return render(screen.chat(), form, None).await;
        }
        DeadlineAction::Days(days) => timezone.in_days(u64::from(days)),
        DeadlineAction::Months(months) => timezone.in_months(months),
        DeadlineAction::Pick(date) => Some(date),
    };
    match date.and_then(|date| timezone.end_of(date)) {
        Some(until) => apply_date(screen, form, until).await,
        None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
    }
}

async fn apply_date(
    screen: Screen<'_>,
    mut form: MakeupForm,
    until: DateTime<Utc>,
) -> HandlerResult {
    let result = form.apply_date(until, Utc::now());
    render_input_result(screen, form, result).await
}

pub(crate) async fn submit_text(step: TextStep<'_>, mut form: MakeupForm) -> HandlerResult {
    let Some(text) = require_text(step.bot(), step.message()).await? else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let scored_scale = form
        .current_scale()
        .map(|scale| scale.grading)
        .filter(|grading| grading.is_scored());
    let accepts_text = match form.step {
        MakeupStep::Date | MakeupStep::Attempts => true,
        MakeupStep::Scale(_) => scored_scale.is_some(),
        MakeupStep::Summary => false,
    };
    if !accepts_text {
        return render(screen.chat(), form, Some(texts::CHOOSE_BUTTON)).await;
    }
    detach(screen.bot(), screen.chat_id(), form.message_id.take()).await?;
    if form.step == MakeupStep::Date {
        return match screen.timezone().parse_deadline(text) {
            Some(until) => apply_date(screen, form, until).await,
            None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
        };
    }
    let Some(value) = parse_number(text) else {
        return render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await;
    };
    let result = match scored_scale {
        Some(grading) => MakeupTerms::scored(value, grading)
            .map_err(MakeupInputError::Domain)
            .and_then(|terms| form.answer(terms)),
        None => form.apply_attempts(value),
    };
    render_input_result(screen, form, result).await
}

pub(crate) async fn back(screen: Screen<'_>, mut form: MakeupForm) -> HandlerResult {
    if !form.back() {
        return cancel(screen, form).await;
    }
    render(screen.chat(), form, None).await
}

async fn cancel(screen: Screen<'_>, form: MakeupForm) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    abandon(screen.bot(), screen.chat_id(), &form).await?;
    show_course_assignments_page(screen, form.course_id, PageRequest::default()).await
}

async fn save(screen: Screen<'_>, form: MakeupForm) -> HandlerResult {
    let Some(command) = form.command(screen.actor(), Utc::now()) else {
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
    reset_dialogue(screen.dialogue()).await?;
    detach(screen.bot(), screen.chat_id(), form.message_id).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), notice)
        .reply_markup(teacher_keyboard())
        .await?;
    show_course_assignments_page(screen, form.course_id, PageRequest::default()).await
}
