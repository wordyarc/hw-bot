use chrono::{DateTime, Utc};
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::types::{ChatId, MessageId};
use teloxide::{ApiError, RequestError};

use super::courses::show_assignment;
use crate::app::{AppError, ChangeTerms, CreateAssignment, ErrorCode, VersionedAssignment};
use crate::bot::assignment_form::{
    AssignmentTerms, CompleteTerms, InputMode, TermsForm, TermsStep, TermsTarget,
};
use crate::bot::callbacks::{AmountChoice, LatePolicyChoice, TermsAction};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::handlers::input::{parse_number, require_text};
use crate::bot::handlers::{reset_dialogue, run_menu_action};
use crate::bot::state::{BotState, FlowState};
use crate::bot::views::{MenuAction, teacher_keyboard, terms_exit_keyboard, terms_form_view};
use crate::bot::{Bot, HandlerResult, assignment_files, texts};
use crate::domain::{
    AssignmentId, AssignmentStatus, DomainError, MAX_LATE_PENALTY, MAX_LATE_PERIOD_DAYS,
    MAX_LATE_PERIODS, TelegramUserId,
};

pub(crate) async fn begin_terms_edit(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if state
        .managing_assignment(message_id, assignment_id)
        .is_none()
    {
        return screen.stale().await;
    }
    let VersionedAssignment {
        assignment,
        revision,
    } = screen
        .services()
        .assignments
        .find_versioned_for_teacher(screen.actor(), assignment_id)
        .await?;
    if assignment.status() == AssignmentStatus::Archived {
        return send_error_code(
            screen.bot(),
            screen.chat_id(),
            ErrorCode::AssignmentUnavailable,
        )
        .await;
    }
    let mut form = TermsForm::new(
        AssignmentTerms::editing(&assignment, revision),
        TermsStep::Summary,
    );
    form.message_id = Some(message_id);
    render(screen.chat(), form, None).await
}

pub(super) async fn render(
    chat: Chat<'_>,
    mut form: TermsForm,
    error: Option<&str>,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    if form.step == TermsStep::Summary {
        form.go(TermsStep::Summary);
    }
    let Some((mut text, keyboard)) = terms_form_view(&form, chat.timezone()) else {
        return discard(chat, form).await;
    };
    if let Some(error) = error {
        text = format!("{error}\n\n{text}");
    }
    match form.mode {
        InputMode::Manual => {
            detach(bot, chat_id, form.message_id.take()).await?;
            chat.ask(&text).await?;
        }
        InputMode::Buttons | InputMode::Calendar(_) => {
            if let Some(id) = form.message_id {
                match bot
                    .edit_message_text(chat_id, id, &text)
                    .reply_markup(keyboard.clone())
                    .await
                {
                    Ok(_) | Err(RequestError::Api(ApiError::MessageNotModified)) => {}
                    Err(RequestError::Api(
                        ApiError::MessageToEditNotFound | ApiError::MessageCantBeEdited,
                    )) => form.message_id = None,
                    Err(error) => return Err(error.into()),
                }
            }
            if form.message_id.is_none() {
                form.message_id = Some(
                    bot.send_message(chat_id, text)
                        .reply_markup(keyboard)
                        .await?
                        .id,
                );
            }
        }
    }
    chat.dialogue()
        .update(BotState::Flow(FlowState::ConfiguringTerms(form)))
        .await?;
    Ok(())
}

/// Сюда попадает только форма, которую нельзя показать: шаг требует значений, которых в ней нет.
/// Каждая правка формы проверяется, поэтому это ошибка программы, а не ввода.
async fn discard(chat: Chat<'_>, form: TermsForm) -> HandlerResult {
    tracing::error!(step = ?form.step, "terms form does not hold what its step needs");
    reset_dialogue(chat.dialogue()).await?;
    detach(chat.bot(), chat.chat_id(), form.message_id).await?;
    chat.bot()
        .send_message(chat.chat_id(), texts::INVALID_TERMS)
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}

async fn detach(bot: &Bot, chat_id: ChatId, message_id: Option<MessageId>) -> HandlerResult {
    let Some(message_id) = message_id else {
        return Ok(());
    };
    match bot.edit_message_reply_markup(chat_id, message_id).await {
        Ok(_)
        | Err(RequestError::Api(
            ApiError::MessageNotModified
            | ApiError::MessageToEditNotFound
            | ApiError::MessageCantBeEdited,
        )) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn validation_message(error: &DomainError) -> String {
    match error {
        DomainError::LateHorizonTooLong { max_days } => texts::horizon_too_long(*max_days),
        DomainError::AssignmentClosingOutOfRange => texts::HORIZON_OUT_OF_RANGE.into(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt => {
            texts::DEADLINE_AFTER_CREATION.into()
        }
        DomainError::InvalidLatePolicy { max, .. } => texts::number_prompt(*max),
        _ => texts::INVALID_TERMS.into(),
    }
}

pub(crate) async fn handle_late_policy(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: LatePolicyChoice,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, TermsStep::Policy) else {
        return screen.stale().await;
    };
    if !form.terms.grading().is_scored() {
        return screen.stale().await;
    }
    let previous = std::mem::replace(
        &mut form.terms.allow_late,
        choice == LatePolicyChoice::Penalty,
    );
    // Прежние штраф и число периодов сохраняются, поэтому возвращённая политика может сразу
    // вести к итогу. Старый горизонт длиннее нынешнего предела здесь допустим: его исправят
    // с итога.
    if form.terms.cannot_be_summarised() {
        form.terms.allow_late = previous;
        return render(screen.chat(), form, Some(texts::HORIZON_OUT_OF_RANGE)).await;
    }
    form.advance();
    render(screen.chat(), form, None).await
}

pub(crate) async fn handle_penalty(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    handle_amount(screen, message_id, choice, state, TermsStep::Penalty).await
}

pub(crate) async fn handle_periods(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    handle_amount(screen, message_id, choice, state, TermsStep::Periods).await
}

async fn handle_amount(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
    step: TermsStep,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, step) else {
        return screen.stale().await;
    };
    match choice {
        AmountChoice::Preset(value) => apply_number(screen, form, value).await,
        AmountChoice::Manual => {
            form.mode = InputMode::Manual;
            render(screen.chat(), form, None).await
        }
    }
}

pub(crate) async fn handle_period(
    screen: Screen<'_>,
    message_id: MessageId,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, TermsStep::Penalty) else {
        return screen.stale().await;
    };
    form.go(TermsStep::Period);
    render(screen.chat(), form, None).await
}

async fn apply_number(screen: Screen<'_>, mut form: TermsForm, value: u16) -> HandlerResult {
    let mut terms = form.terms.clone();
    let max = match form.step {
        TermsStep::Penalty => {
            terms.penalty = Some(value);
            MAX_LATE_PENALTY
        }
        TermsStep::Period => {
            terms.period_days = value;
            MAX_LATE_PERIOD_DAYS
        }
        TermsStep::Periods => {
            terms.max_periods = Some(value);
            MAX_LATE_PERIODS
        }
        _ => return screen.stale().await,
    };
    if !(1..=max).contains(&value) {
        return render(screen.chat(), form, Some(&texts::number_prompt(max))).await;
    }
    if let Err(error) = terms.validate() {
        return render(screen.chat(), form, Some(&validation_message(&error))).await;
    }
    form.terms = terms;
    form.advance();
    render(screen.chat(), form, None).await
}

pub(super) async fn apply_date(
    screen: Screen<'_>,
    mut form: TermsForm,
    at: DateTime<Utc>,
) -> HandlerResult {
    if form.step == TermsStep::DurationDate {
        let Some(deadline) = form.terms.deadline_at else {
            return render(screen.chat(), form, None).await;
        };
        let policy = form.terms.preview_policy(1).map_err(AppError::from)?;
        let periods = match u16::try_from(policy.periods_to_close_at(deadline, at)) {
            Ok(periods) if (1..=MAX_LATE_PERIODS).contains(&periods) => periods,
            _ => return render(screen.chat(), form, Some(texts::INVALID_DURATION_DATE)).await,
        };
        let mut terms = form.terms.clone();
        terms.max_periods = Some(periods);
        if let Err(error) = terms.validate() {
            let message = match error {
                DomainError::LateHorizonTooLong { .. } | DomainError::InvalidLatePolicy { .. } => {
                    texts::INVALID_DURATION_DATE.to_owned()
                }
                _ => validation_message(&error),
            };
            return render(screen.chat(), form, Some(&message)).await;
        }
        form.go(TermsStep::DurationConfirm {
            selected: at,
            periods,
        });
    } else {
        if matches!(form.terms.target, TermsTarget::Create(_)) && at <= Utc::now() {
            return render(screen.chat(), form, Some(texts::DEADLINE_IN_FUTURE)).await;
        }
        let mut terms = form.terms.clone();
        terms.deadline_at = Some(at);
        if let Err(error) = terms.validate() {
            return render(screen.chat(), form, Some(&validation_message(&error))).await;
        }
        form.terms = terms;
        form.advance();
    }
    render(screen.chat(), form, None).await
}

pub(crate) async fn submit_terms(step: TextStep<'_>, mut form: TermsForm) -> HandlerResult {
    let Some(text) = require_text(step.bot(), step.message()).await? else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    if !matches!(
        form.step,
        TermsStep::Deadline
            | TermsStep::DurationDate
            | TermsStep::Penalty
            | TermsStep::Period
            | TermsStep::Periods
    ) {
        return render(screen.chat(), form, Some(texts::CHOOSE_BUTTON)).await;
    }
    detach(screen.bot(), screen.chat_id(), form.message_id.take()).await?;
    match form.step {
        TermsStep::Deadline | TermsStep::DurationDate => {
            match screen.timezone().parse_deadline(text) {
                Some(at) => apply_date(screen, form, at).await,
                None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
            }
        }
        _ => match parse_number(text) {
            Some(value) => apply_number(screen, form, value).await,
            None => render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await,
        },
    }
}

pub(crate) async fn back(screen: Screen<'_>, mut form: TermsForm) -> HandlerResult {
    match (form.mode, form.step) {
        (InputMode::Manual, step) if step != TermsStep::Period => form.mode = InputMode::Buttons,
        (InputMode::Calendar(_), _) => form.mode = InputMode::Buttons,
        _ => {
            let Some(step) = form.previous_step() else {
                return cancel(screen, form).await;
            };
            // Если настройку штрафа прервали, итог снова запрещает опоздание, но введённые числа сохраняет.
            if step == TermsStep::Summary && form.terms.complete().is_none() {
                form.terms.allow_late = false;
            }
            form.go(step);
        }
    }
    render(screen.chat(), form, None).await
}

pub(crate) async fn handle_terms(
    screen: Screen<'_>,
    message_id: MessageId,
    action: TermsAction,
    state: BotState,
) -> HandlerResult {
    if let BotState::Flow(FlowState::ConfirmingTermsExit {
        message_id: current,
        mut form,
        action: menu,
        telegram_user_id,
    }) = state
    {
        if current != message_id {
            return screen.stale().await;
        }
        return match action {
            TermsAction::Stay => {
                form.message_id = Some(current);
                render(screen.chat(), form, None).await
            }
            TermsAction::Exit => {
                reset_dialogue(screen.dialogue()).await?;
                abandon(screen.bot(), screen.chat_id(), &form).await?;
                run_menu_action(screen.chat(), telegram_user_id, menu).await
            }
            _ => screen.stale().await,
        };
    }
    let Some(active) = state.active_terms() else {
        return screen.stale().await;
    };
    let expected_step = active.step;
    let Some(mut form) = state.terms_form(message_id, expected_step) else {
        return screen.stale().await;
    };
    if action == TermsAction::Back {
        if !form.can_go_back() {
            return screen.stale().await;
        }
        return back(screen, form).await;
    }
    if action == TermsAction::DurationDate && form.step == TermsStep::Periods {
        form.go(TermsStep::DurationDate);
        return render(screen.chat(), form, None).await;
    }
    if let TermsStep::DurationConfirm { periods, .. } = form.step
        && action == TermsAction::DurationApply
    {
        form.terms.max_periods = Some(periods);
        form.advance();
        return render(screen.chat(), form, None).await;
    }
    if form.step != TermsStep::Summary {
        return screen.stale().await;
    }
    let Some(complete) = form.terms.complete() else {
        return render(screen.chat(), form, None).await;
    };
    form.previous_closes_at.get_or_insert(complete.closes_at);
    match action {
        TermsAction::Save => return save(screen, form, complete).await,
        TermsAction::Cancel => return cancel(screen, form).await,
        _ => {
            let Some(step) = TermsStep::summary_edit(action, &form.terms) else {
                return screen.stale().await;
            };
            form.go(step);
        }
    }
    form.return_to_summary = true;
    render(screen.chat(), form, None).await
}

pub(crate) async fn confirm_exit(
    screen: Screen<'_>,
    mut form: TermsForm,
    action: MenuAction,
    telegram_user_id: TelegramUserId,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), form.message_id).await?;
    let sent = screen
        .bot()
        .send_message(screen.chat_id(), texts::TERMS_EXIT_QUESTION)
        .reply_markup(terms_exit_keyboard())
        .await?;
    form.message_id = Some(sent.id);
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::ConfirmingTermsExit {
            message_id: sent.id,
            form,
            action,
            telegram_user_id,
        }))
        .await?;
    Ok(())
}

pub(crate) async fn abandon(bot: &Bot, chat_id: ChatId, form: &TermsForm) -> HandlerResult {
    detach(bot, chat_id, form.message_id).await?;
    let text = if matches!(form.terms.target, TermsTarget::Create(_)) {
        texts::DRAFT_DISCARDED
    } else {
        texts::TERMS_DISCARDED
    };
    bot.send_message(chat_id, text)
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}

async fn cancel(screen: Screen<'_>, form: TermsForm) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    abandon(screen.bot(), screen.chat_id(), &form).await?;
    if let TermsTarget::Edit { assignment_id, .. } = form.terms.target {
        show_assignment(screen, assignment_id).await?;
    }
    Ok(())
}

async fn save(screen: Screen<'_>, form: TermsForm, complete: CompleteTerms) -> HandlerResult {
    if let Err(error) = form.terms.validate() {
        return render(screen.chat(), form, Some(&validation_message(&error))).await;
    }
    let TermsForm {
        terms, message_id, ..
    } = form;
    let assignments = &screen.services().assignments;
    let (result, edited) = match terms.target {
        TermsTarget::Create(draft) => {
            let created = assignments
                .create(CreateAssignment {
                    teaching_course_id: draft.text.course_id,
                    actor: screen.actor(),
                    title: draft.text.title,
                    body: draft.text.body,
                    max_attempts: draft.max_attempts,
                    grading: draft.grading,
                    late_policy: complete.late_policy,
                    created_at: Utc::now(),
                    deadline_at: complete.deadline_at,
                })
                .await;
            (created, None)
        }
        TermsTarget::Edit {
            assignment_id,
            revision,
            ..
        } => {
            let changed = assignments
                .change_terms(ChangeTerms {
                    assignment_id,
                    actor: screen.actor(),
                    deadline_at: complete.deadline_at,
                    late_policy: complete.late_policy,
                    expected_revision: revision,
                })
                .await;
            (changed, Some(assignment_id))
        }
    };
    let assignment = match (result, edited) {
        (Ok(assignment), _) => assignment,
        (Err(error), Some(assignment_id)) if error.code() == ErrorCode::ConcurrentModification => {
            return conflict(screen, message_id, assignment_id).await;
        }
        (Err(error), _) => return Err(error.into()),
    };
    reset_dialogue(screen.dialogue()).await?;
    detach(screen.bot(), screen.chat_id(), message_id).await?;
    let notice = if edited.is_some() {
        texts::TERMS_SAVED
    } else {
        texts::DRAFT_CREATED
    };
    screen
        .bot()
        .send_message(screen.chat_id(), notice)
        .reply_markup(teacher_keyboard())
        .await?;
    if edited.is_some() {
        show_assignment(screen, assignment.id()).await
    } else {
        assignment_files::show(screen, assignment.id(), true).await
    }
}

async fn conflict(
    screen: Screen<'_>,
    message_id: Option<MessageId>,
    assignment_id: AssignmentId,
) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    detach(screen.bot(), screen.chat_id(), message_id).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), texts::TERMS_CONFLICT)
        .reply_markup(teacher_keyboard())
        .await?;
    show_assignment(screen, assignment_id).await
}
