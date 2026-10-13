use std::borrow::Cow;

use chrono::{DateTime, NaiveDate, Utc};
use teloxide::payloads::{EditMessageTextSetters, SendMessageSetters};
use teloxide::types::{ChatId, InlineKeyboardMarkup, MessageId};
use teloxide::{ApiError, RequestError};

use super::deadline::{DateChoice, choose_date};
use crate::bot::callbacks::{AmountChoice, DeadlineAction, ReplayableAction};
use crate::bot::context::{Chat, Screen, TextStep};
use crate::bot::form::{Form, InputMode};
use crate::bot::handlers::input::require_text;
use crate::bot::handlers::{handle_replayable_action, reset_dialogue};
use crate::bot::state::BotState;
use crate::bot::timezone::BotTimezone;
use crate::bot::views::teacher_keyboard;
use crate::bot::{Bot, HandlerResult, texts};
use crate::domain::DomainError;

pub(super) trait FormScreen: Form + Sized {
    fn view(
        &mut self,
        timezone: &BotTimezone,
        now: DateTime<Utc>,
    ) -> Option<(String, InlineKeyboardMarkup)>;
    fn into_state(self) -> BotState;
    fn abandoned_text(&self) -> &'static str;
    fn input_message(error: Self::InputError) -> Option<String>;
    fn cancelled_to(&self) -> Option<ReplayableAction>;
}

pub(super) async fn render<F: FormScreen>(
    chat: Chat<'_>,
    mut form: F,
    error: Option<&str>,
) -> HandlerResult {
    let Some((mut text, keyboard)) = form.view(chat.timezone(), chat.now()) else {
        return discard(chat, form).await;
    };
    if let Some(error) = error {
        text = format!("{error}\n\n{text}");
    }
    let mode = form.mode();
    show_step(chat, form.message_id_mut(), mode, text, keyboard).await?;
    chat.dialogue().update(form.into_state()).await?;
    Ok(())
}

pub(super) async fn render_input_result<F: FormScreen>(
    screen: Screen<'_>,
    form: F,
    result: Result<(), F::InputError>,
) -> HandlerResult {
    let Err(error) = result else {
        return render(screen.chat(), form, None).await;
    };
    match F::input_message(error) {
        Some(message) => render(screen.chat(), form, Some(&message)).await,
        None => screen.stale().await,
    }
}

pub(super) async fn input_manually<F: FormScreen>(chat: Chat<'_>, mut form: F) -> HandlerResult {
    form.set_mode(InputMode::Manual);
    render(chat, form, None).await
}

pub(super) async fn choose_amount<F: FormScreen>(
    screen: Screen<'_>,
    mut form: F,
    choice: AmountChoice,
    apply: impl FnOnce(&mut F, u16) -> Result<(), F::InputError>,
) -> HandlerResult {
    match choice {
        AmountChoice::Preset(value) => {
            let result = apply(&mut form, value);
            render_input_result(screen, form, result).await
        }
        AmountChoice::Manual => input_manually(screen.chat(), form).await,
    }
}

pub(super) fn form_error(error: &DomainError) -> String {
    match error {
        DomainError::LateHorizonTooLong { max_days } => texts::horizon_too_long(*max_days),
        DomainError::MakeupUntilOutOfRange { max_days } => {
            texts::makeup_until_out_of_range(*max_days)
        }
        DomainError::AssignmentClosingOutOfRange => texts::HORIZON_OUT_OF_RANGE.into(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt => {
            texts::DEADLINE_AFTER_CREATION.into()
        }
        DomainError::InvalidLatePolicy { max, .. }
        | DomainError::InvalidMakeupAttempts { max, .. }
        | DomainError::InvalidMakeupCredit { max, .. } => texts::number_prompt(*max),
        DomainError::MakeupTermsDoNotFitGrading => texts::CHOOSE_BUTTON.into(),
        _ => texts::INVALID_TERMS.into(),
    }
}

pub(super) async fn take_text<'a, F: FormScreen>(
    step: TextStep<'a>,
    mut form: F,
) -> HandlerResult<Option<(Screen<'a>, F, Cow<'a, str>)>> {
    let Some(text) = require_text(step.bot(), step.message()).await? else {
        return Ok(None);
    };
    let Some(screen) = step.resolve().await? else {
        return Ok(None);
    };
    if !form.accepts_text() {
        render(screen.chat(), form, Some(texts::CHOOSE_BUTTON)).await?;
        return Ok(None);
    }
    detach(screen.bot(), screen.chat_id(), form.message_id_mut().take()).await?;
    Ok(Some((screen, form, text)))
}

pub(super) async fn handle_date<F: FormScreen>(
    screen: Screen<'_>,
    mut form: F,
    action: DeadlineAction,
    calendar_from: NaiveDate,
) -> HandlerResult {
    match choose_date(action, screen.timezone(), screen.now(), calendar_from) {
        DateChoice::Mode(mode) => {
            form.set_mode(mode);
            render(screen.chat(), form, None).await
        }
        DateChoice::At(Some(at)) => apply_date(screen, form, at).await,
        DateChoice::At(None) => render(screen.chat(), form, Some(texts::DATE_FORMAT)).await,
    }
}

pub(super) async fn apply_date<F: FormScreen>(
    screen: Screen<'_>,
    mut form: F,
    at: DateTime<Utc>,
) -> HandlerResult {
    let result = form.apply_date(at, screen.now());
    render_input_result(screen, form, result).await
}

pub(super) async fn step_back<F: FormScreen>(screen: Screen<'_>, mut form: F) -> HandlerResult {
    if !form.step_back() {
        return screen.stale().await;
    }
    render(screen.chat(), form, None).await
}

pub(super) async fn back<F: FormScreen>(screen: Screen<'_>, mut form: F) -> HandlerResult {
    if !form.back() {
        return cancel(screen, form).await;
    }
    render(screen.chat(), form, None).await
}

pub(super) async fn cancel<F: FormScreen>(screen: Screen<'_>, form: F) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    abandon(screen.bot(), screen.chat_id(), &form).await?;
    match form.cancelled_to() {
        Some(action) => handle_replayable_action(screen, action, screen.now()).await,
        None => Ok(()),
    }
}

pub(super) async fn abandon<F: FormScreen>(bot: &Bot, chat_id: ChatId, form: &F) -> HandlerResult {
    detach(bot, chat_id, form.message_id()).await?;
    bot.send_message(chat_id, form.abandoned_text())
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}

/// Сюда попадает только форма, которую нельзя показать: шаг требует значений, которых в ней нет.
/// Каждая правка формы проверяется, поэтому это ошибка программы, а не ввода.
pub(super) async fn discard<F: FormScreen>(chat: Chat<'_>, form: F) -> HandlerResult {
    tracing::error!(step = ?form.step(), "form does not hold what its step needs");
    reset_dialogue(chat.dialogue()).await?;
    abandon(chat.bot(), chat.chat_id(), &form).await
}

pub(super) async fn finish(
    screen: Screen<'_>,
    message_id: Option<MessageId>,
    notice: &str,
) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    detach(screen.bot(), screen.chat_id(), message_id).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), notice)
        .reply_markup(teacher_keyboard())
        .await?;
    Ok(())
}

async fn show_step(
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
