use chrono::{DateTime, Utc};
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::types::{ChatId, InlineKeyboardMarkup, MessageId};
use teloxide::{ApiError, RequestError};

use super::courses::show_assignment;
use crate::app::{ChangeTerms, CreateAssignment, ErrorCode, VersionedAssignment};
use crate::bot::assignment_form::{
    AssignmentTerms, CompleteTerms, InputMode, TermsForm, TermsInputError, TermsStep, TermsTarget,
};
use crate::bot::callbacks::{AmountChoice, LatePolicyChoice, MakeupChoice, TermsAction};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::errors::send_error_code;
use crate::bot::handlers::input::{parse_number, require_text};
use crate::bot::handlers::{reset_dialogue, run_menu_action};
use crate::bot::state::{BotState, FlowState};
use crate::bot::views::{MenuAction, teacher_keyboard, terms_exit_keyboard, terms_form_view};
use crate::bot::{Bot, HandlerResult, assignment_files, texts};
use crate::domain::{AssignmentId, AssignmentStatus, DomainError, MakeupTerms, TelegramUserId};

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

pub(super) async fn render(
    chat: Chat<'_>,
    mut form: TermsForm,
    error: Option<&str>,
) -> HandlerResult {
    if form.step == TermsStep::Summary {
        form.go(TermsStep::Summary);
    }
    let Some((mut text, keyboard)) = terms_form_view(&form, chat.timezone(), Utc::now()) else {
        return discard(chat, form).await;
    };
    if let Some(error) = error {
        text = format!("{error}\n\n{text}");
    }
    show_step(chat, &mut form.message_id, form.mode, text, keyboard).await?;
    chat.dialogue()
        .update(BotState::Flow(FlowState::ConfiguringTerms(form)))
        .await?;
    Ok(())
}

pub(super) async fn show_step(
    chat: Chat<'_>,
    message_id: &mut Option<MessageId>,
    mode: InputMode,
    text: String,
    keyboard: InlineKeyboardMarkup,
) -> HandlerResult {
    let bot = chat.bot();
    let chat_id = chat.chat_id();
    if mode == InputMode::Manual {
        detach(bot, chat_id, message_id.take()).await?;
        return chat.ask(&text).await;
    }
    if let Some(id) = *message_id {
        match bot
            .edit_message_text(chat_id, id, &text)
            .reply_markup(keyboard.clone())
            .await
        {
            Ok(_) | Err(RequestError::Api(ApiError::MessageNotModified)) => {}
            Err(RequestError::Api(
                ApiError::MessageToEditNotFound | ApiError::MessageCantBeEdited,
            )) => *message_id = None,
            Err(error) => return Err(error.into()),
        }
    }
    if message_id.is_none() {
        *message_id = Some(
            bot.send_message(chat_id, text)
                .reply_markup(keyboard)
                .await?
                .id,
        );
    }
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

pub(super) async fn detach(
    bot: &Bot,
    chat_id: ChatId,
    message_id: Option<MessageId>,
) -> HandlerResult {
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

pub(super) async fn apply_date(
    screen: Screen<'_>,
    mut form: TermsForm,
    at: DateTime<Utc>,
) -> HandlerResult {
    let result = form.apply_date(at, Utc::now());
    render_input_result(screen, form, result).await
}

async fn render_input_result(
    screen: Screen<'_>,
    form: TermsForm,
    result: Result<(), TermsInputError>,
) -> HandlerResult {
    let message = match result {
        Ok(()) => return render(screen.chat(), form, None).await,
        Err(TermsInputError::UnexpectedStep) => return screen.stale().await,
        Err(TermsInputError::NumberOutOfRange { max }) => texts::number_prompt(max),
        Err(TermsInputError::InvalidDurationDate) => texts::INVALID_DURATION_DATE.into(),
        Err(TermsInputError::DeadlineInPast) => texts::DEADLINE_IN_FUTURE.into(),
        Err(TermsInputError::Domain(error)) => validation_message(&error),
    };
    render(screen.chat(), form, Some(&message)).await
}

pub(crate) async fn submit_terms(step: TextStep<'_>, mut form: TermsForm) -> HandlerResult {
    let Some(text) = require_text(step.bot(), step.message()).await? else {
        return Ok(());
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let accepts_text = match form.step {
        TermsStep::Deadline
        | TermsStep::DurationDate
        | TermsStep::Penalty
        | TermsStep::Period
        | TermsStep::Periods => true,
        TermsStep::Makeup => form.terms.grading().is_scored(),
        TermsStep::Policy | TermsStep::DurationConfirm { .. } | TermsStep::Summary => false,
    };
    if !accepts_text {
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
        TermsStep::Makeup => match parse_number(text) {
            Some(value) => {
                let result = MakeupTerms::scored(value, form.terms.grading())
                    .map_err(TermsInputError::Domain)
                    .and_then(|makeup| form.choose_makeup(makeup));
                render_input_result(screen, form, result).await
            }
            None => render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await,
        },
        _ => match parse_number(text) {
            Some(value) => apply_number(screen, form, value).await,
            None => render(screen.chat(), form, Some(texts::INTEGER_REQUIRED)).await,
        },
    }
}

pub(crate) async fn back(screen: Screen<'_>, mut form: TermsForm) -> HandlerResult {
    if !form.back() {
        return cancel(screen, form).await;
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
