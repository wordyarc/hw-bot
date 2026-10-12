use chrono::{DateTime, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::types::{InlineKeyboardMarkup, MessageId};

use super::courses::show_assignment;
use super::form::{
    FormScreen, abandon, apply_date, cancel, detach, finish, render, render_input_result,
    step_back, take_text,
};
use crate::app::{ChangeTerms, CreateAssignment, ErrorCode, VersionedAssignment};
use crate::bot::assignment_form::{
    AssignmentTerms, CompleteTerms, TermsForm, TermsInputError, TermsStep, TermsTarget,
};
use crate::bot::callbacks::{
    AmountChoice, LatePolicyChoice, MakeupChoice, ReplayableAction, TermsAction,
};
use crate::bot::context::{Screen, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::form::InputMode;
use crate::bot::handlers::input::parse_step_number;
use crate::bot::handlers::{reset_dialogue, run_menu_action};
use crate::bot::state::{BotState, FlowState};
use crate::bot::timezone::BotTimezone;
use crate::bot::views::{MenuAction, terms_exit_keyboard, terms_form_view};
use crate::bot::{HandlerResult, assignment_files, texts};
use crate::domain::{AssignmentId, AssignmentStatus, DomainError, MakeupTerms, TelegramUserId};

impl FormScreen for TermsForm {
    fn view(
        &mut self,
        timezone: &BotTimezone,
        now: DateTime<Utc>,
    ) -> Option<(String, InlineKeyboardMarkup)> {
        if self.step == TermsStep::Summary {
            self.go(TermsStep::Summary);
        }
        terms_form_view(self, timezone, now)
    }

    fn into_state(self) -> BotState {
        BotState::Flow(FlowState::ConfiguringTerms(self))
    }

    fn abandoned_text(&self) -> &'static str {
        match self.terms.target {
            TermsTarget::Create(_) => texts::DRAFT_DISCARDED,
            TermsTarget::Edit { .. } => texts::TERMS_DISCARDED,
        }
    }

    fn input_message(error: TermsInputError) -> Option<String> {
        Some(match error {
            TermsInputError::UnexpectedStep => return None,
            TermsInputError::NumberOutOfRange { max } => texts::number_prompt(max),
            TermsInputError::InvalidDurationDate => texts::INVALID_DURATION_DATE.into(),
            TermsInputError::DeadlineInPast => texts::DEADLINE_IN_FUTURE.into(),
            TermsInputError::Domain(error) => validation_message(&error),
        })
    }

    fn cancelled_to(&self) -> Option<ReplayableAction> {
        match self.terms.target {
            TermsTarget::Create(_) => None,
            TermsTarget::Edit { assignment_id, .. } => {
                Some(ReplayableAction::ManageAssignment(assignment_id))
            }
        }
    }
}

pub(crate) async fn begin_terms_edit(
    screen: Screen<'_>,
    message_id: MessageId,
    assignment_id: AssignmentId,
    state: &BotState,
) -> HandlerResult {
    if !state.is_managing_assignment(message_id, assignment_id) {
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
    let question = screen
        .services()
        .courses
        .makeup_question(
            screen.actor(),
            assignment.teaching_course_id(),
            assignment.grading(),
            Some(assignment.id()),
        )
        .await?;
    let mut form = TermsForm::new(
        AssignmentTerms::editing(&assignment, revision, question),
        TermsStep::Summary,
    );
    form.message_id = Some(message_id);
    render(screen.chat(), form, None).await
}

fn validation_message(error: &DomainError) -> String {
    match error {
        DomainError::LateHorizonTooLong { max_days } => texts::horizon_too_long(*max_days),
        DomainError::AssignmentClosingOutOfRange => texts::HORIZON_OUT_OF_RANGE.into(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt => {
            texts::DEADLINE_AFTER_CREATION.into()
        }
        DomainError::InvalidLatePolicy { max, .. }
        | DomainError::InvalidMakeupCredit { max, .. } => texts::number_prompt(*max),
        DomainError::MakeupTermsDoNotFitGrading => texts::CHOOSE_BUTTON.into(),
        _ => texts::INVALID_TERMS.into(),
    }
}

pub(crate) async fn handle_makeup(
    screen: Screen<'_>,
    message_id: MessageId,
    choice: MakeupChoice,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, TermsStep::Makeup) else {
        return screen.stale().await;
    };
    let makeup = match choice {
        MakeupChoice::Manual => {
            form.mode = InputMode::Manual;
            return render(screen.chat(), form, None).await;
        }
        MakeupChoice::NotOffered => Ok(MakeupTerms::NotOffered),
        MakeupChoice::PassFail => Ok(MakeupTerms::PassFail),
        MakeupChoice::Credit(credit) => MakeupTerms::scored(credit, form.terms.grading()),
    };
    let result = makeup
        .map_err(TermsInputError::Domain)
        .and_then(|makeup| form.choose_makeup(makeup));
    render_input_result(screen, form, result).await
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
    let result = form.choose_policy(choice.deducted_from());
    render_input_result(screen, form, result).await
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

pub(crate) async fn handle_duration_periods(
    screen: Screen<'_>,
    message_id: MessageId,
    periods: u16,
    state: BotState,
) -> HandlerResult {
    let Some(mut form) = state.terms_form(message_id, TermsStep::DurationDate) else {
        return screen.stale().await;
    };
    let result = form.apply_duration_periods(periods);
    render_input_result(screen, form, result).await
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
    let result = form.apply_number(value);
    render_input_result(screen, form, result).await
}

pub(crate) async fn submit_terms(step: TextStep<'_>, form: TermsForm) -> HandlerResult {
    let Some((screen, mut form, text)) = take_text(step, form).await? else {
        return Ok(());
    };
    match form.step {
        TermsStep::Deadline | TermsStep::DurationDate => {
            match screen.timezone().parse_deadline(&text) {
                Some(at) => apply_date(screen, form, at).await,
                None => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
            }
        }
        TermsStep::Makeup => match parse_step_number(&text) {
            Some(value) => {
                let result = MakeupTerms::scored(value, form.terms.grading())
                    .map_err(TermsInputError::Domain)
                    .and_then(|makeup| form.choose_makeup(makeup));
                render_input_result(screen, form, result).await
            }
            None => render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await,
        },
        _ => match parse_step_number(&text) {
            Some(value) => apply_number(screen, form, value).await,
            None => render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await,
        },
    }
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
        return step_back(screen, form).await;
    }
    if action == TermsAction::DurationDate && form.step == TermsStep::Periods {
        form.go(TermsStep::DurationDate);
        return render(screen.chat(), form, None).await;
    }
    if action == TermsAction::DurationApply && form.confirm_duration() {
        return render(screen.chat(), form, None).await;
    }
    if form.step != TermsStep::Summary {
        return screen.stale().await;
    }
    let Some(complete) = form.terms.complete() else {
        return render(screen.chat(), form, None).await;
    };
    match action {
        TermsAction::Save => return save(screen, form, complete).await,
        TermsAction::Cancel => return cancel(screen, form).await,
        _ => {
            if !form.edit_summary(action) {
                return screen.stale().await;
            }
        }
    }
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

async fn save(screen: Screen<'_>, form: TermsForm, complete: CompleteTerms) -> HandlerResult {
    if let Err(error) = form.validate_for_save(Utc::now()) {
        let message = TermsForm::input_message(error);
        return render(screen.chat(), form, message.as_deref()).await;
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
                    student_group_ids: Some(draft.groups.ids()),
                    max_attempts: draft.max_attempts,
                    grading: draft.grading,
                    late_policy: complete.late_policy,
                    makeup: terms.makeup,
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
                    makeup: terms.makeup,
                    expected_revision: revision,
                })
                .await;
            (changed, Some(assignment_id))
        }
    };
    let assignment = match (result, edited) {
        (Ok(assignment), _) => assignment,
        (Err(error), Some(assignment_id)) if error.code() == ErrorCode::ConcurrentModification => {
            finish(screen, message_id, texts::TERMS_CONFLICT).await?;
            return show_assignment(screen, assignment_id).await;
        }
        (Err(error), _) => return Err(error.into()),
    };
    match edited {
        Some(_) => {
            finish(screen, message_id, texts::TERMS_SAVED).await?;
            show_assignment(screen, assignment.id()).await
        }
        None => {
            finish(screen, message_id, texts::DRAFT_CREATED).await?;
            assignment_files::show(screen, assignment.id(), true).await
        }
    }
}
