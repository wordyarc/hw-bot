use chrono::{DateTime, Utc};
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::types::{ChatId, MessageId};

use crate::app::{ChangeTerms, CreateAssignment, ErrorCode};
use crate::bot::callbacks::{AmountChoice, LatePolicyChoice, TermsAction};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::handlers::input::{parse_number, require_text};
use crate::bot::handlers::{reset_dialogue, resolve_actor, run_menu_action};
use crate::bot::state::{AssignmentTerms, BotState, FlowState, TermsForm, TermsStep, TermsTarget};
use crate::bot::views::{MenuAction, teacher_keyboard, terms_form_view};
use crate::bot::{Bot, HandlerResult, assignment_files, texts};
use crate::domain::{AssignmentId, AssignmentStatus, DomainError, DomainResult, LatePolicy};

use super::courses::show_assignment;

pub(in crate::bot::handlers) async fn begin_edit(
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
    let (assignment, revision) = match screen
        .context
        .services
        .assignments
        .find_versioned_for_teacher(screen.actor, assignment_id)
        .await
    {
        Ok(value) => value,
        Err(error) => return screen.fail(error).await,
    };
    if assignment.status() == AssignmentStatus::Archived {
        return send_error_code(screen.bot, screen.chat_id, ErrorCode::AssignmentUnavailable).await;
    }
    let policy = assignment.late_policy();
    let terms = AssignmentTerms {
        target: TermsTarget::Edit {
            assignment_id,
            grading: assignment.grading(),
            revision,
            created_at: assignment.created_at(),
            original_closes_at: assignment.closes_at(),
        },
        deadline_at: Some(assignment.deadline_at()),
        allow_late: policy.is_some(),
        period_days: policy.map_or(7, LatePolicy::period_days),
        penalty: policy.map(LatePolicy::penalty),
        max_periods: policy.map(LatePolicy::max_periods),
    };
    let mut form = TermsForm::new(terms, TermsStep::Summary);
    form.message_id = Some(message_id);
    render(screen.chat(), form, None).await
}

pub(super) async fn render(
    chat: Chat<'_>,
    mut form: TermsForm,
    error: Option<&str>,
) -> HandlerResult {
    let Chat { bot, chat_id, deps } = chat;
    if form.step == TermsStep::Summary {
        next(&mut form);
    }
    let (mut text, keyboard) = terms_form_view(&form, &deps.context.timezone);
    if let Some(error) = error {
        text = format!("{error}\n\n{text}");
    }
    if form.manual {
        detach(bot, chat_id, &mut form).await?;
        chat.ask(&text).await?;
    } else if let Some(id) = form.message_id {
        match bot
            .edit_message_text(chat_id, id, text)
            .reply_markup(keyboard)
            .await
        {
            Ok(_) | Err(teloxide::RequestError::Api(teloxide::ApiError::MessageNotModified)) => {}
            Err(error) => return Err(error.into()),
        }
    } else {
        form.message_id = Some(
            bot.send_message(chat_id, text)
                .reply_markup(keyboard)
                .await?
                .id,
        );
    }
    deps.dialogue
        .update(BotState::Flow(FlowState::ConfiguringTerms(form)))
        .await?;
    Ok(())
}

pub(super) async fn detach(bot: &Bot, chat_id: ChatId, form: &mut TermsForm) -> HandlerResult {
    if let Some(id) = form.message_id.take() {
        bot.edit_message_reply_markup(chat_id, id).await?;
    }
    Ok(())
}

pub(super) fn go(form: &mut TermsForm, step: TermsStep) {
    let step = if step == TermsStep::Summary {
        form.terms.next_step()
    } else {
        step
    };
    form.step = step;
    if step == TermsStep::Summary {
        form.return_to_summary = false;
    }
    form.manual = step == TermsStep::Period;
    form.calendar_month = None;
}

fn next(form: &mut TermsForm) {
    let step = form.terms.next_step();
    go(form, step);
}

fn validate_partial(terms: &AssignmentTerms) -> DomainResult<()> {
    if let Some(deadline) = terms.deadline_at {
        if let TermsTarget::Edit { created_at, .. } = terms.target
            && deadline <= created_at
        {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }
        if terms.allow_late {
            LatePolicy::new(
                terms.period_days,
                terms.penalty.unwrap_or(1),
                terms.max_periods.unwrap_or(1),
            )?
            .validate_horizon(deadline)?;
        }
    }
    Ok(())
}

pub(super) fn validation_message(error: &DomainError) -> String {
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

pub(in crate::bot::handlers) async fn handle_late_policy(
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
    form.terms.allow_late = choice == LatePolicyChoice::Penalty;
    next(&mut form);
    render(screen.chat(), form, None).await
}

pub(in crate::bot::handlers) async fn handle_penalty(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: AmountChoice,
    state: BotState,
) -> HandlerResult {
    handle_amount(screen, message_id, choice, state, TermsStep::Penalty).await
}

pub(in crate::bot::handlers) async fn handle_periods(
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
            form.manual = true;
            render(screen.chat(), form, None).await
        }
    }
}

pub(in crate::bot::handlers) async fn handle_period(
    screen: Screen<'_>,
    message_id: MessageId,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, TermsStep::Penalty) else {
        return screen.stale().await;
    };
    go(&mut form, TermsStep::Period);
    render(screen.chat(), form, None).await
}

async fn apply_number(screen: Screen<'_>, mut form: TermsForm, value: u16) -> HandlerResult {
    let mut terms = form.terms.clone();
    let max = match form.step {
        TermsStep::Penalty => {
            terms.penalty = Some(value);
            crate::domain::MAX_LATE_PENALTY
        }
        TermsStep::Period => {
            terms.period_days = value;
            crate::domain::MAX_LATE_PERIOD_DAYS
        }
        TermsStep::Periods => {
            terms.max_periods = Some(value);
            crate::domain::MAX_LATE_PERIODS
        }
        _ => return screen.stale().await,
    };
    if !(1..=max).contains(&value) {
        return render(screen.chat(), form, Some(&texts::number_prompt(max))).await;
    }
    if let Err(error) = validate_partial(&terms) {
        return render(screen.chat(), form, Some(&validation_message(&error))).await;
    }
    form.terms = terms;
    if form.return_to_summary {
        next(&mut form);
    } else {
        let step = match form.step {
            TermsStep::Period => TermsStep::Penalty,
            TermsStep::Penalty => TermsStep::Periods,
            _ => TermsStep::Summary,
        };
        go(&mut form, step);
    }
    render(screen.chat(), form, None).await
}

pub(super) async fn apply_date(
    screen: Screen<'_>,
    mut form: TermsForm,
    at: DateTime<Utc>,
) -> HandlerResult {
    if form.step == TermsStep::DurationDate {
        let deadline = form.terms.deadline_at.expect("duration follows deadline");
        let policy = LatePolicy::new(form.terms.period_days, form.terms.penalty.unwrap_or(1), 1)
            .map_err(crate::app::AppError::from)?;
        let periods = policy.periods_until(deadline, at);
        if periods == 0 || periods > u32::from(crate::domain::MAX_LATE_PERIODS) {
            return render(screen.chat(), form, Some(texts::INVALID_DURATION_DATE)).await;
        }
        let periods = u16::try_from(periods).expect("bounded by MAX_LATE_PERIODS");
        let mut terms = form.terms.clone();
        terms.max_periods = Some(periods);
        if let Err(error) = validate_partial(&terms) {
            let message = match error {
                DomainError::LateHorizonTooLong { .. } | DomainError::InvalidLatePolicy { .. } => {
                    texts::INVALID_DURATION_DATE.to_owned()
                }
                _ => validation_message(&error),
            };
            return render(screen.chat(), form, Some(&message)).await;
        }
        go(
            &mut form,
            TermsStep::DurationConfirm {
                selected: at,
                periods,
            },
        );
    } else {
        if matches!(form.terms.target, TermsTarget::Create(_)) && at <= Utc::now() {
            return render(screen.chat(), form, Some(texts::DEADLINE_IN_FUTURE)).await;
        }
        let mut terms = form.terms.clone();
        terms.deadline_at = Some(at);
        if let Err(error) = validate_partial(&terms) {
            return render(screen.chat(), form, Some(&validation_message(&error))).await;
        }
        form.terms = terms;
        if form.return_to_summary || !form.terms.allow_late {
            next(&mut form);
        } else {
            go(&mut form, TermsStep::Penalty);
        }
    }
    render(screen.chat(), form, None).await
}

pub(in crate::bot::handlers) async fn submit_terms(
    step: TextStep<'_>,
    mut form: TermsForm,
) -> HandlerResult {
    let Some(text) = require_text(step.bot, step.message).await? else {
        return Ok(());
    };
    let Some(actor) = resolve_actor(
        step.bot,
        step.message.chat.id,
        step.telegram_user_id,
        &step.deps.context.services,
        step.deps.dialogue,
    )
    .await?
    else {
        return Ok(());
    };
    let screen = step.screen(actor);
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
    detach(screen.bot, screen.chat_id, &mut form).await?;
    match form.step {
        TermsStep::Deadline | TermsStep::DurationDate => {
            match screen.context.timezone.parse_deadline(text) {
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

pub(in crate::bot::handlers) async fn back(
    screen: Screen<'_>,
    mut form: TermsForm,
) -> HandlerResult {
    if form.manual && form.step != TermsStep::Period {
        form.manual = false;
    } else if form.calendar_month.take().is_some() {
    } else if form.return_to_summary && form.step != TermsStep::Summary {
        // Отмена недособранного включения штрафа возвращает строгий итог; введённые числа остаются.
        if form.terms.late_policy().is_err() {
            form.terms.allow_late = false;
        }
        go(&mut form, TermsStep::Summary);
    } else {
        let step = match form.step {
            TermsStep::Policy => return cancel(screen, form).await,
            TermsStep::Deadline if !form.terms.grading().is_scored() => {
                return cancel(screen, form).await;
            }
            TermsStep::Deadline => TermsStep::Policy,
            TermsStep::Penalty => TermsStep::Deadline,
            TermsStep::Period | TermsStep::Periods => TermsStep::Penalty,
            TermsStep::DurationDate => TermsStep::Periods,
            TermsStep::DurationConfirm { .. } => TermsStep::DurationDate,
            TermsStep::Summary if matches!(form.terms.target, TermsTarget::Edit { .. }) => {
                return cancel(screen, form).await;
            }
            TermsStep::Summary if form.terms.allow_late => TermsStep::Periods,
            TermsStep::Summary => TermsStep::Deadline,
        };
        go(&mut form, step);
    }
    render(screen.chat(), form, None).await
}

pub(in crate::bot::handlers) async fn handle_terms(
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
                abandon(screen.bot, screen.chat_id, &form).await?;
                reset_dialogue(screen.dialogue).await?;
                run_menu_action(
                    screen.bot,
                    screen.chat_id,
                    telegram_user_id,
                    menu,
                    screen.deps(),
                )
                .await
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
        go(&mut form, TermsStep::DurationDate);
        return render(screen.chat(), form, None).await;
    }
    if let TermsStep::DurationConfirm { periods, .. } = form.step
        && action == TermsAction::DurationApply
    {
        form.terms.max_periods = Some(periods);
        next(&mut form);
        return render(screen.chat(), form, None).await;
    }
    if form.step != TermsStep::Summary {
        return screen.stale().await;
    }
    if form.previous_closes_at.is_none() {
        form.previous_closes_at = form.terms.deadline_at.map(|deadline| {
            form.terms
                .late_policy()
                .expect("complete summary")
                .map_or(deadline, |policy| {
                    policy.closes_at(deadline).expect("validated summary")
                })
        });
    }
    match action {
        TermsAction::Save => return save(screen, form).await,
        TermsAction::Cancel => return cancel(screen, form).await,
        TermsAction::Deadline => go(&mut form, TermsStep::Deadline),
        TermsAction::Policy if form.terms.grading().is_scored() => go(&mut form, TermsStep::Policy),
        TermsAction::Penalty if form.terms.allow_late => go(&mut form, TermsStep::Penalty),
        TermsAction::Period if form.terms.allow_late => go(&mut form, TermsStep::Period),
        TermsAction::Periods if form.terms.allow_late => go(&mut form, TermsStep::Periods),
        _ => return screen.stale().await,
    }
    form.return_to_summary = true;
    render(screen.chat(), form, None).await
}

pub(in crate::bot::handlers) async fn confirm_exit(
    screen: Screen<'_>,
    mut form: TermsForm,
    action: MenuAction,
    telegram_user_id: crate::domain::TelegramUserId,
) -> HandlerResult {
    detach(screen.bot, screen.chat_id, &mut form).await?;
    let sent = screen
        .bot
        .send_message(screen.chat_id, texts::TERMS_EXIT_QUESTION)
        .reply_markup(crate::bot::views::terms_exit_keyboard())
        .await?;
    form.message_id = Some(sent.id);
    screen
        .dialogue
        .update(BotState::Flow(FlowState::ConfirmingTermsExit {
            message_id: sent.id,
            form,
            action,
            telegram_user_id,
        }))
        .await?;
    Ok(())
}

pub(in crate::bot::handlers) async fn abandon(
    bot: &Bot,
    chat_id: ChatId,
    form: &TermsForm,
) -> HandlerResult {
    let mut form = form.clone();
    detach(bot, chat_id, &mut form).await?;
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
    reset_dialogue(screen.dialogue).await?;
    abandon(screen.bot, screen.chat_id, &form).await?;
    if let TermsTarget::Edit { assignment_id, .. } = form.terms.target {
        show_assignment(screen, assignment_id).await?;
    }
    Ok(())
}

async fn save(screen: Screen<'_>, form: TermsForm) -> HandlerResult {
    let terms = &form.terms;
    let Some(deadline_at) = terms.deadline_at else {
        return screen.stale().await;
    };
    if terms.next_step() != TermsStep::Summary {
        return render(screen.chat(), form, None).await;
    }
    if let Err(error) = validate_partial(terms) {
        return render(screen.chat(), form, Some(&validation_message(&error))).await;
    }
    let late_policy = match terms.late_policy() {
        Ok(policy) => policy,
        Err(error) => {
            return render(screen.chat(), form, Some(&validation_message(&error))).await;
        }
    };
    let creating = matches!(terms.target, TermsTarget::Create(_));
    let result = match &terms.target {
        TermsTarget::Create(draft) => {
            screen
                .context
                .services
                .assignments
                .create(CreateAssignment {
                    teaching_course_id: draft.course_id,
                    actor: screen.actor,
                    title: draft.title.clone(),
                    body: draft.body.clone(),
                    max_attempts: draft.max_attempts,
                    grading: draft.grading,
                    late_policy,
                    created_at: Utc::now(),
                    deadline_at,
                })
                .await
        }
        TermsTarget::Edit {
            assignment_id,
            revision,
            ..
        } => {
            screen
                .context
                .services
                .assignments
                .change_terms(ChangeTerms {
                    assignment_id: *assignment_id,
                    actor: screen.actor,
                    deadline_at,
                    late_policy,
                    expected_revision: *revision,
                })
                .await
        }
    };
    let assignment = match result {
        Ok(value) => value,
        Err(error) => return screen.fail(error).await,
    };
    reset_dialogue(screen.dialogue).await?;
    let mut form = form;
    detach(screen.bot, screen.chat_id, &mut form).await?;
    screen
        .bot
        .send_message(
            screen.chat_id,
            if creating {
                texts::DRAFT_CREATED
            } else {
                texts::TERMS_SAVED
            },
        )
        .reply_markup(teacher_keyboard())
        .await?;
    if creating {
        assignment_files::show(screen, assignment.id(), true).await
    } else {
        show_assignment(screen, assignment.id()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::state::DraftAssignment;
    use crate::domain::{Grading, TeachingCourseId};

    #[test]
    fn summary_transition_collects_missing_policy_fields_without_defaulting_them() {
        let mut draft =
            DraftAssignment::new(TeachingCourseId::new(), "Title".into(), "Body".into(), 1);
        draft.grading = Grading::scored(5).unwrap();
        let mut terms = AssignmentTerms::creating(draft);
        terms.allow_late = true;
        let mut form = TermsForm::new(terms, TermsStep::Policy);

        go(&mut form, TermsStep::Summary);
        assert!(form.step == TermsStep::Deadline);
        form.terms.deadline_at = Some(crate::test_support::t0());
        go(&mut form, TermsStep::Summary);
        assert!(form.step == TermsStep::Penalty);
        assert!(form.terms.penalty.is_none());
        form.terms.penalty = Some(2);
        go(&mut form, TermsStep::Summary);
        assert!(form.step == TermsStep::Periods);
        assert!(form.terms.max_periods.is_none());
        form.terms.max_periods = Some(4);
        go(&mut form, TermsStep::Summary);
        assert!(form.step == TermsStep::Summary);
        assert_eq!(
            form.terms.late_policy().unwrap(),
            Some(LatePolicy::new(7, 2, 4).unwrap())
        );
    }
}
