use std::collections::HashMap;
use std::num::{NonZeroU32, NonZeroUsize};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use futures::{FutureExt, Stream, StreamExt, future};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::{self, InMemStorage};
use teloxide::prelude::*;
use teloxide::stop::StopToken;
use teloxide::types::{AllowedUpdate, UpdateId};
use teloxide::update_listeners::{AsUpdateStream, Polling, UpdateListener};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::app::{ApplicationContainer, ErrorCode};
use crate::domain::TelegramUserId;
use crate::health::ComponentHealth;
use crate::invites::CourseInviteCodec;

pub const MAX_CONCURRENT_UPDATES: usize = 1024;

mod assignment_files;
mod assignment_form;
pub mod callbacks;
mod clock;
mod context;
mod errors;
mod files;
mod form;
mod handlers;
mod invites;
mod makeup_form;
mod state;
pub mod texts;
mod timezone;
mod views;

use crate::telegram::network::Bot;
use crate::telegram::{is_invalid_token, new_bot};
pub use clock::Clock;
pub use errors::app_error_message;
use files::FileTransfers;
use handlers::home::default_commands;
use handlers::{Command, handle_callback, handle_command, handle_message, reset_dialogue};
use invites::BotInvites;
use state::{BotDialogue, BotState};
pub use timezone::BotTimezone;

use context::BotContext;
use errors::BotError;

type HandlerResult<T = ()> = Result<T, BotError>;

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    polling_bot: teloxide::Bot,
    application: ApplicationContainer,
    settings: TelegramBotSettings,
    timezone: BotTimezone,
    clock: Clock,
    admission: Arc<UpdateAdmission>,
    file_transfers: FileTransfers,
    codec: CourseInviteCodec,
}

#[derive(Clone, Copy, Debug)]
pub struct TelegramBotSettings {
    pub request_timeout: Duration,
    pub polling_timeout: Duration,
    pub rate_limit_replenish: Duration,
    pub rate_limit_burst: NonZeroU32,
    pub max_concurrent_updates: NonZeroUsize,
    pub file_transfer_timeout: Duration,
    pub max_concurrent_file_transfers: NonZeroUsize,
}

struct UpdateAdmission {
    rate_limiter: DefaultKeyedRateLimiter<TelegramUserId>,
    semaphore: Arc<Semaphore>,
    checks: AtomicU64,
    rate_limit_rejections: AtomicU64,
    permits: Mutex<HashMap<UpdateId, OwnedSemaphorePermit>>,
}

struct AdmissionListener<L> {
    inner: L,
    admission: Arc<UpdateAdmission>,
    invalid_credentials: Arc<AtomicBool>,
}

#[derive(Debug, Error)]
pub enum TelegramBotError {
    #[error("Telegram credentials were rejected")]
    InvalidCredentials,

    #[error("Telegram bot startup request failed: {0}")]
    Startup(&'static str),

    #[error("Telegram bot has no username, course invite links cannot be built")]
    MissingBotUsername,

    #[error("Telegram dispatcher stopped unexpectedly")]
    UnexpectedTermination,
}

impl TelegramBot {
    pub fn new(
        application: ApplicationContainer,
        token: impl Into<String>,
        api_base_url: &str,
        settings: TelegramBotSettings,
        timezone: BotTimezone,
        clock: Clock,
        codec: CourseInviteCodec,
    ) -> anyhow::Result<Self> {
        let quota = Quota::with_period(settings.rate_limit_replenish)
            .context("Telegram rate limit replenishment must be positive")?
            .allow_burst(settings.rate_limit_burst);
        let polling_http_timeout = settings
            .polling_timeout
            .checked_add(settings.request_timeout)
            .context("Telegram polling HTTP timeout is too large")?;
        let token = token.into();
        let bot = new_bot(token.clone(), api_base_url, settings.request_timeout)?;
        let polling_bot = new_bot(token.clone(), api_base_url, polling_http_timeout)?;
        let transfer_bot = new_bot(token, api_base_url, settings.file_transfer_timeout)?;
        let admission = Arc::new(UpdateAdmission {
            rate_limiter: RateLimiter::keyed(quota),
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent_updates.get())),
            checks: AtomicU64::new(0),
            rate_limit_rejections: AtomicU64::new(0),
            permits: Mutex::new(HashMap::new()),
        });

        Ok(Self {
            bot: Bot::new(bot, settings.request_timeout),
            polling_bot,
            application,
            settings,
            timezone,
            clock,
            admission,
            file_transfers: FileTransfers::new(
                transfer_bot,
                settings.max_concurrent_file_transfers.get(),
                settings.file_transfer_timeout,
            ),
            codec,
        })
    }

    async fn startup(&self, cancellation: &CancellationToken) -> Result<BotInvites, Interrupted> {
        let me = required(cancellation, self.bot.get_me().send()).await?;
        let bot_username = me
            .user
            .username
            .ok_or(Interrupted::Failed(TelegramBotError::MissingBotUsername))?;
        let webhook = required(cancellation, self.polling_bot.get_webhook_info().send()).await?;
        if webhook.url.is_some() {
            required(cancellation, self.polling_bot.delete_webhook().send()).await?;
        }

        let commands = self.bot.set_my_commands(default_commands()).send();
        if let Err(error) = attempted(cancellation, commands).await? {
            errors::log_request_error("set_my_commands", &error);
        }
        let menu = self
            .bot
            .set_chat_menu_button()
            .menu_button(teloxide::types::MenuButton::Commands)
            .send();
        if let Err(error) = attempted(cancellation, menu).await? {
            errors::log_request_error("set_chat_menu_button", &error);
        }
        Ok(BotInvites::new(self.codec.clone(), bot_username))
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        health: ComponentHealth,
    ) -> Result<(), TelegramBotError> {
        let invites = match self.startup(&cancellation).await {
            Ok(invites) => invites,
            Err(Interrupted::Cancelled) => return Ok(()),
            Err(Interrupted::Failed(error)) => return Err(error),
        };
        let _ready = health.ready();
        let storage = InMemStorage::<BotState>::new();
        let force_stop = CancellationToken::new();
        let _stop_handlers_on_drop = force_stop.clone().drop_guard();
        let invalid_credentials = Arc::new(AtomicBool::new(false));
        let handler_credentials = invalid_credentials.clone();
        let stop = cancellation.child_token();
        let handler_stop = stop.clone();
        let mut dispatcher = Dispatcher::builder(self.bot.inner.clone(), schema(force_stop))
            .dependencies(dptree::deps![
                self.bot,
                storage,
                BotContext {
                    services: self.application,
                    timezone: self.timezone,
                    clock: self.clock,
                    invites,
                    transfers: self.file_transfers,
                },
                self.admission.clone()
            ])
            .distribution_function(|update| update.chat().map(|chat| chat.id))
            .worker_queue_size(self.settings.max_concurrent_updates.get())
            .default_handler(|_| async {})
            .error_handler(Arc::new(move |error: BotError| {
                let credentials = handler_credentials.clone();
                let stop = handler_stop.clone();
                async move {
                    if error.is_invalid_token() {
                        credentials.store(true, Ordering::Release);
                        stop.cancel();
                    }
                }
            }))
            .build();
        let listener = Polling::builder(self.polling_bot)
            .timeout(self.settings.polling_timeout)
            .build();
        let listener = admitted_listener(
            listener,
            self.admission.clone(),
            invalid_credentials.clone(),
        );
        let shutdown = dispatcher.shutdown_token();
        let dispatch = dispatcher.try_dispatch_with_listener(
            listener,
            Arc::new(|error: teloxide::RequestError| async move {
                errors::log_request_error("get_updates", &error);
            }),
        );
        tokio::pin!(dispatch);

        tokio::select! {
            result = &mut dispatch => result.map_err(map_startup_error)?,
            () = stop.cancelled() => {
                // Ожидание dispatch запустило бы ещё idle dispatcher даже после отмены.
                if shutdown.shutdown().is_ok() {
                    dispatch.await.map_err(map_startup_error)?;
                }
                return if invalid_credentials.load(Ordering::Acquire) {
                    Err(TelegramBotError::InvalidCredentials)
                } else { Ok(()) };
            }
        }

        if invalid_credentials.load(Ordering::Acquire) {
            Err(TelegramBotError::InvalidCredentials)
        } else {
            Err(TelegramBotError::UnexpectedTermination)
        }
    }
}

enum Interrupted {
    Cancelled,
    Failed(TelegramBotError),
}

async fn attempted<T>(
    cancellation: &CancellationToken,
    request: impl Future<Output = Result<T, teloxide::RequestError>>,
) -> Result<Result<T, teloxide::RequestError>, Interrupted> {
    cancellation
        .run_until_cancelled(request)
        .await
        .ok_or(Interrupted::Cancelled)
}

async fn required<T>(
    cancellation: &CancellationToken,
    request: impl Future<Output = Result<T, teloxide::RequestError>>,
) -> Result<T, Interrupted> {
    attempted(cancellation, request)
        .await?
        .map_err(|error| Interrupted::Failed(map_startup_error(error)))
}

fn map_startup_error(error: teloxide::RequestError) -> TelegramBotError {
    if is_invalid_token(&error) {
        TelegramBotError::InvalidCredentials
    } else {
        errors::log_request_error("startup", &error);
        TelegramBotError::Startup(errors::request_error_kind(&error))
    }
}

async fn admit_update(update: &Update, admission: &UpdateAdmission) -> bool {
    let Some(from) = update.from() else {
        return false;
    };
    let raw_user_id = from.id.0;
    let Some(telegram_user_id) = i64::try_from(raw_user_id)
        .ok()
        .and_then(|id| TelegramUserId::new(id).ok())
    else {
        return false;
    };
    if admission
        .checks
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(1024)
    {
        admission.rate_limiter.retain_recent();
    }
    if admission.rate_limiter.check_key(&telegram_user_id).is_err() {
        log_rate_limit_rejection(admission, telegram_user_id, update.id);
        return false;
    }

    // Нельзя терять update при ожидании permit: новый polling offset подтвердит весь batch.
    let permit = admission
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("Telegram admission semaphore closed");
    admission
        .permits
        .lock()
        .expect("Telegram admission permit mutex poisoned")
        .insert(update.id, permit);

    true
}

fn log_rate_limit_rejection(
    admission: &UpdateAdmission,
    telegram_user_id: TelegramUserId,
    update_id: UpdateId,
) {
    let rejected = admission
        .rate_limit_rejections
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    if rejected.is_power_of_two() {
        tracing::warn!(
            kind = "rate_limit",
            rejected,
            telegram_user_id = telegram_user_id.as_i64(),
            update_id = update_id.0,
            "Telegram update rejected"
        );
    }
}

fn admitted_listener<L>(
    listener: L,
    admission: Arc<UpdateAdmission>,
    invalid_credentials: Arc<AtomicBool>,
) -> AdmissionListener<L>
where
    L: UpdateListener<Err = teloxide::RequestError>,
{
    AdmissionListener {
        inner: listener,
        admission,
        invalid_credentials,
    }
}

impl<'a, L> AsUpdateStream<'a> for AdmissionListener<L>
where
    L: UpdateListener<Err = teloxide::RequestError> + Send + 'a,
    <L as AsUpdateStream<'a>>::Stream: Send,
{
    type StreamErr = teloxide::RequestError;
    type Stream = Pin<Box<dyn Stream<Item = Result<Update, teloxide::RequestError>> + Send + 'a>>;

    fn as_stream(&'a mut self) -> Self::Stream {
        let admission = self.admission.clone();
        let invalid_credentials = self.invalid_credentials.clone();
        Box::pin(
            self.inner
                .as_stream()
                .take_while(move |result| {
                    let keep_polling = !result.as_ref().is_err_and(is_invalid_token);
                    if !keep_polling {
                        invalid_credentials.store(true, Ordering::Release);
                    }
                    future::ready(keep_polling)
                })
                .filter_map(move |result| {
                    let admission = admission.clone();
                    async move {
                        match result {
                            Ok(update) => admit_update(&update, &admission)
                                .await
                                .then_some(Ok(update)),
                            Err(error) => Some(Err(error)),
                        }
                    }
                }),
        )
    }
}

impl<L> UpdateListener for AdmissionListener<L>
where
    L: UpdateListener<Err = teloxide::RequestError> + Send + 'static,
    for<'a> <L as AsUpdateStream<'a>>::Stream: Send,
{
    type Err = L::Err;

    fn stop_token(&mut self) -> StopToken {
        self.inner.stop_token()
    }

    fn hint_allowed_updates(&mut self, hint: &mut dyn Iterator<Item = AllowedUpdate>) {
        self.inner.hint_allowed_updates(hint);
    }
}

fn take_admission_permit(
    update: Update,
    admission: Arc<UpdateAdmission>,
) -> Option<OwnedSemaphorePermit> {
    admission
        .permits
        .lock()
        .expect("Telegram admission permit mutex poisoned")
        .remove(&update.id)
}

fn schema(force_stop: CancellationToken) -> UpdateHandler<BotError> {
    let messages = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(dptree::endpoint(handle_message));
    let callbacks = Update::filter_callback_query().endpoint(handle_callback);

    update_lifecycle(force_stop)
        .filter_map(take_admission_permit)
        .chain(
            dialogue::enter::<Update, InMemStorage<BotState>, BotState, _>()
                .branch(messages)
                .branch(callbacks),
        )
}

fn update_lifecycle(force_stop: CancellationToken) -> UpdateHandler<BotError> {
    use std::ops::ControlFlow;
    use std::panic::AssertUnwindSafe;

    use tracing::Instrument;

    dptree::from_fn(
        move |deps, next| {
            let force_stop = force_stop.clone();
            let update = deps.get::<Update>();
            let kind = match &update.kind {
                teloxide::types::UpdateKind::Message(_) => "message",
                teloxide::types::UpdateKind::CallbackQuery(_) => "callback",
                _ => "other",
            };
            let span = tracing::info_span!(
                "telegram_update",
                update_id = update.id.0,
                chat_id = update.chat().map(|chat| chat.id.0),
                kind,
                action = tracing::field::Empty,
            );
            let recovery = PanicRecovery::of(&deps, &update);
            async move {
                let started = tokio::time::Instant::now();
                tokio::select! {
                    biased;
                    () = force_stop.cancelled() => ControlFlow::Break(Ok(())),
                    // Паника обработчика убивает chat-worker teloxide, а следующий update этого
                    // чата роняет уже диспетчер. Здесь она остаётся отказом одного update.
                    result = AssertUnwindSafe(next(deps)).catch_unwind() => {
                        let result = match result {
                            Ok(result) => result,
                            Err(_) => {
                                tracing::error!("Telegram update handler panicked");
                                recovery.run().await;
                                ControlFlow::Break(Ok(()))
                            }
                        };
                        if let ControlFlow::Break(Err(ref error)) = result {
                            errors::log_handler_error(error);
                        }
                        tracing::debug!(
                            elapsed_ms = started.elapsed().as_millis(),
                            "Telegram update completed"
                        );
                        result
                    }
                }
            }
            .instrument(span)
        },
        dptree::HandlerSignature::Entry,
    )
}

/// Состояние диалога могло и вызвать панику, поэтому оно сбрасывается: иначе каждый следующий
/// ввод в этом чате падал бы на том же месте.
struct PanicRecovery {
    chat: Option<(Arc<Bot>, BotDialogue, ChatId)>,
}

impl PanicRecovery {
    fn of(deps: &DependencyMap, update: &Update) -> Self {
        let chat = update
            .chat()
            .filter(|chat| chat.is_private())
            .and_then(|chat| {
                let bot = deps.try_get::<Bot>()?;
                let storage = deps.try_get::<Arc<InMemStorage<BotState>>>()?;
                let dialogue = BotDialogue::new(Arc::clone(&storage), chat.id);
                Some((bot, dialogue, chat.id))
            });
        Self { chat }
    }

    async fn run(self) {
        let Some((bot, dialogue, chat_id)) = self.chat else {
            return;
        };
        if let Err(error) = reset_dialogue(&dialogue).await {
            errors::log_handler_error(&error);
        }
        if let Err(error) = errors::send_error_code(&bot, chat_id, ErrorCode::InternalError).await {
            errors::log_handler_error(&error);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::bot::state::FlowState;
    use crate::domain::AssignmentId;

    fn admission(burst: u32, concurrency: usize) -> Arc<UpdateAdmission> {
        Arc::new(UpdateAdmission {
            rate_limiter: RateLimiter::keyed(
                Quota::with_period(Duration::from_secs(60))
                    .unwrap()
                    .allow_burst(NonZeroU32::new(burst).unwrap()),
            ),
            semaphore: Arc::new(Semaphore::new(concurrency)),
            checks: AtomicU64::new(0),
            rate_limit_rejections: AtomicU64::new(0),
            permits: Mutex::new(HashMap::new()),
        })
    }

    fn update(id: u32, user: u64) -> Update {
        serde_json::from_str(
            &json!({
                "update_id": id,
                "message": {
                    "message_id": id, "date": 0,
                    "chat": {"id": user, "type": "private"},
                    "from": {"id": user, "is_bot": false, "first_name": "Student"},
                    "text": "/help"
                }
            })
            .to_string(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn updates_above_the_user_burst_are_dropped() {
        let admission = admission(1, 2);
        assert!(admit_update(&update(1, 42), &admission).await);
        assert!(!admit_update(&update(2, 42), &admission).await);
        assert!(take_admission_permit(update(2, 42), admission).is_none());
    }

    #[tokio::test]
    async fn each_user_has_an_independent_rate_limit_bucket() {
        let admission = admission(1, 2);
        assert!(admit_update(&update(1, 42), &admission).await);
        assert!(admit_update(&update(2, 43), &admission).await);
    }

    #[tokio::test]
    async fn waiting_updates_are_admitted_only_after_the_handler_releases_its_permit() {
        let admission = admission(2, 1);
        let first = update(1, 42);
        let second = update(2, 43);
        assert!(admit_update(&first, &admission).await);
        let mut waiting = std::pin::pin!(admit_update(&second, &admission));
        assert!(futures::poll!(waiting.as_mut()).is_pending());

        let permit = take_admission_permit(first, admission.clone()).unwrap();
        assert!(futures::poll!(waiting.as_mut()).is_pending());
        drop(permit);

        assert!(waiting.await);
        assert!(take_admission_permit(second.clone(), admission.clone()).is_some());
    }

    #[tokio::test]
    async fn a_handler_panic_fails_one_update_and_resets_the_chat() {
        let telegram = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:test/SendMessage"))
            .and(body_partial_json(json!({
                "chat_id": 42,
                "text": app_error_message(ErrorCode::InternalError),
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {
                    "message_id": 1,
                    "date": 0,
                    "chat": { "id": 42, "type": "private" },
                    "from": { "id": 999, "is_bot": true, "first_name": "Test Bot" },
                    "text": "response"
                }
            })))
            .expect(1)
            .mount(&telegram)
            .await;
        let timeout = Duration::from_secs(5);
        let bot = Bot::new(
            new_bot("123:test", &telegram.uri(), timeout).unwrap(),
            timeout,
        );
        let storage = InMemStorage::<BotState>::new();
        let dialogue = BotDialogue::new(storage.clone(), ChatId(42));
        dialogue
            .update(BotState::Flow(FlowState::AwaitingSubmission {
                assignment_id: AssignmentId::new(),
                makeup: false,
            }))
            .await
            .unwrap();
        let update = json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "date": 0,
                "chat": { "id": 42, "type": "private" },
                "from": { "id": 42, "is_bot": false, "first_name": "Student" },
                "text": "boom"
            }
        });
        let update: Update = serde_json::from_str(&update.to_string()).unwrap();
        assert!(update.chat().is_some());
        let handler =
            update_lifecycle(CancellationToken::new()).endpoint(|update: Update| async move {
                assert_eq!(update.id.0, 0, "handler bug");
                HandlerResult::Ok(())
            });

        let outcome = handler.dispatch(dptree::deps![update, bot, storage]).await;

        assert!(matches!(outcome, ControlFlow::Break(Ok(()))));
        assert!(dialogue.get().await.unwrap().is_none());
    }
}
