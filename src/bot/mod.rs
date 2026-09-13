use anyhow::Context;
use futures::{Stream, StreamExt, future};
use governor::{DefaultKeyedRateLimiter, Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::{self, InMemStorage};
use teloxide::prelude::*;
use teloxide::stop::StopToken;
use teloxide::types::{AllowedUpdate, UpdateId};
use teloxide::update_listeners::{AsUpdateStream, Polling, UpdateListener};
use thiserror::Error;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::app::ApplicationContainer;
use crate::domain::TelegramUserId;
use crate::health::ComponentHealth;
use crate::invites::GroupInviteCodec;

pub const MAX_CONCURRENT_UPDATES: usize = 1024;

mod assignment_files;
pub mod callbacks;
mod context;
mod errors;
mod files;
mod handlers;
mod invites;
mod state;
pub mod texts;
mod timezone;
mod views;

use crate::telegram::network::Bot;
pub use errors::app_error_message;
use files::FileTransfers;
use handlers::home::default_commands;
use handlers::{Command, handle_callback, handle_command, handle_message};
use invites::BotInvites;
use state::BotState;
pub use timezone::BotTimezone;

use context::BotContext;
use errors::BotError;

type HandlerResult<T = ()> = Result<T, BotError>;
type BotServices = ApplicationContainer;

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    polling_bot: teloxide::Bot,
    application: ApplicationContainer,
    settings: TelegramBotSettings,
    timezone: BotTimezone,
    admission: UpdateAdmission,
    file_transfers: FileTransfers,
    codec: GroupInviteCodec,
}

#[derive(Clone, Copy, Debug)]
pub struct TelegramBotSettings {
    pub request_timeout: Duration,
    pub polling_timeout: Duration,
    pub rate_limit_replenish: Duration,
    pub rate_limit_burst: u32,
    pub max_concurrent_updates: usize,
    pub file_transfer_timeout: Duration,
    pub max_concurrent_file_transfers: usize,
}

#[derive(Clone)]
struct UpdateAdmission {
    rate_limiter: Arc<DefaultKeyedRateLimiter<TelegramUserId>>,
    semaphore: Arc<Semaphore>,
    checks: Arc<AtomicU64>,
    rate_limit_rejections: Arc<AtomicU64>,
    permits: Arc<Mutex<HashMap<UpdateId, OwnedSemaphorePermit>>>,
}

struct AdmissionListener<L> {
    inner: L,
    admission: UpdateAdmission,
    invalid_credentials: Arc<AtomicBool>,
}

#[derive(Debug, Error)]
pub enum TelegramBotError {
    #[error("Telegram credentials were rejected")]
    InvalidCredentials,

    #[error("Telegram bot startup request failed: {0}")]
    Startup(&'static str),

    #[error("Telegram bot has no username, group invite links cannot be built")]
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
        codec: GroupInviteCodec,
    ) -> anyhow::Result<Self> {
        let burst = NonZeroU32::new(settings.rate_limit_burst)
            .context("Telegram rate limit burst must be positive")?;
        let quota = Quota::with_period(settings.rate_limit_replenish)
            .context("Telegram rate limit replenishment must be positive")?
            .allow_burst(burst);
        let polling_http_timeout = settings
            .polling_timeout
            .checked_add(settings.request_timeout)
            .context("Telegram polling HTTP timeout is too large")?;
        let token = token.into();
        let bot = crate::telegram::new_bot(token.clone(), api_base_url, settings.request_timeout)?;
        let polling_bot =
            crate::telegram::new_bot(token.clone(), api_base_url, polling_http_timeout)?;
        let transfer_bot =
            crate::telegram::new_bot(token, api_base_url, settings.file_transfer_timeout)?;
        let admission = UpdateAdmission {
            rate_limiter: Arc::new(RateLimiter::keyed(quota)),
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent_updates)),
            checks: Arc::new(AtomicU64::new(0)),
            rate_limit_rejections: Arc::new(AtomicU64::new(0)),
            permits: Arc::new(Mutex::new(HashMap::new())),
        };

        Ok(Self {
            bot: Bot::new(bot, settings.request_timeout),
            polling_bot,
            application,
            settings,
            timezone,
            admission,
            file_transfers: FileTransfers::new(
                transfer_bot,
                settings.max_concurrent_file_transfers,
                settings.file_transfer_timeout,
            ),
            codec,
        })
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        health: ComponentHealth,
    ) -> Result<(), TelegramBotError> {
        let Some(me) = cancellation
            .run_until_cancelled(self.bot.get_me().send())
            .await
        else {
            return Ok(());
        };
        let bot_username = me
            .map_err(map_startup_error)?
            .user
            .username
            .ok_or(TelegramBotError::MissingBotUsername)?;
        let invites = BotInvites::new(self.codec, bot_username);
        let Some(webhook) = cancellation
            .run_until_cancelled(self.polling_bot.get_webhook_info().send())
            .await
        else {
            return Ok(());
        };
        if webhook.map_err(map_startup_error)?.url.is_some() {
            let Some(result) = cancellation
                .run_until_cancelled(self.polling_bot.delete_webhook().send())
                .await
            else {
                return Ok(());
            };
            result.map_err(map_startup_error)?;
        }
        if !announce_commands(&self.bot, &cancellation).await {
            return Ok(());
        }
        let _ready = health.ready();
        let storage = InMemStorage::<BotState>::new();
        let force_stop = CancellationToken::new();
        let _stop_handlers_on_drop = force_stop.clone().drop_guard();
        let invalid_credentials = Arc::new(AtomicBool::new(false));
        let handler_credentials = invalid_credentials.clone();
        let handler_cancellation = cancellation.clone();
        let mut dispatcher = Dispatcher::builder(self.bot.inner.clone(), schema(force_stop))
            .dependencies(dptree::deps![
                self.bot,
                storage,
                BotContext {
                    services: self.application,
                    timezone: self.timezone,
                    invites,
                },
                self.admission.clone(),
                self.file_transfers
            ])
            .distribution_function(|update| update.chat().map(|chat| chat.id))
            .worker_queue_size(self.settings.max_concurrent_updates)
            .default_handler(|_| async {})
            .error_handler(Arc::new(move |error: BotError| {
                let credentials = handler_credentials.clone();
                let cancellation = handler_cancellation.clone();
                async move {
                    if matches!(
                        error.request_error(),
                        Some(teloxide::RequestError::Api(
                            teloxide::ApiError::InvalidToken
                        ))
                    ) {
                        credentials.store(true, Ordering::Release);
                        cancellation.cancel();
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
            () = cancellation.cancelled() => {
                // До запуска dispatcher ещё idle: ждать dispatch при отмене нельзя — он запустится.
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

async fn announce_commands(bot: &Bot, cancellation: &CancellationToken) -> bool {
    let Some(commands) = cancellation
        .run_until_cancelled(bot.set_my_commands(default_commands()).send())
        .await
    else {
        return false;
    };
    if let Err(error) = commands {
        errors::log_request_error("set_my_commands", &error);
    }

    let Some(menu) = cancellation
        .run_until_cancelled(
            bot.set_chat_menu_button()
                .menu_button(teloxide::types::MenuButton::Commands)
                .send(),
        )
        .await
    else {
        return false;
    };
    if let Err(error) = menu {
        errors::log_request_error("set_chat_menu_button", &error);
    }

    true
}

fn map_startup_error(error: teloxide::RequestError) -> TelegramBotError {
    if matches!(
        error,
        teloxide::RequestError::Api(teloxide::ApiError::InvalidToken)
    ) {
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

    // Ждём, а не отбрасываем: следующий polling offset подтвердит Telegram весь batch.
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
    admission: UpdateAdmission,
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
                    let keep_polling = !matches!(
                        result,
                        Err(teloxide::RequestError::Api(
                            teloxide::ApiError::InvalidToken
                        ))
                    );
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
    admission: UpdateAdmission,
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
            async move {
                let started = tokio::time::Instant::now();
                tokio::select! {
                    biased;
                    () = force_stop.cancelled() => ControlFlow::Break(Ok(())),
                    result = next(deps) => {
                        if let ControlFlow::Break(Err(ref error)) = result {
                            errors::log_handler_error(error);
                        }
                        tracing::debug!(elapsed_ms = started.elapsed().as_millis(), "Telegram update completed");
                        result
                    }
                }
            }
            .instrument(span)
        },
        dptree::HandlerSignature::Entry,
    )
}
