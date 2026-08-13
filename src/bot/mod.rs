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
use teloxide::error_handlers::LoggingErrorHandler;
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

const MAX_CONCURRENT_UPDATES: usize = 1024;

mod assignment_files;
mod callbacks;
mod errors;
mod files;
mod handlers;
mod state;
mod timezone;
mod views;

use files::FileTransfers;
use handlers::{Command, default_commands, handle_callback, handle_command, handle_message};
use state::BotState;
pub use timezone::BotTimezone;

type HandlerResult<T = ()> = anyhow::Result<T>;
type BotServices = ApplicationContainer;

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    polling_bot: Bot,
    application: ApplicationContainer,
    settings: TelegramBotSettings,
    timezone: BotTimezone,
    admission: UpdateAdmission,
    file_transfers: FileTransfers,
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
    concurrency_rejections: Arc<AtomicU64>,
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
    Startup(#[source] teloxide::RequestError),

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
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !settings.request_timeout.is_zero(),
            "Telegram request timeout must be positive"
        );
        anyhow::ensure!(
            !settings.polling_timeout.is_zero(),
            "Telegram polling timeout must be positive"
        );
        anyhow::ensure!(
            !settings.rate_limit_replenish.is_zero(),
            "Telegram rate limit replenishment must be positive"
        );
        anyhow::ensure!(
            settings.max_concurrent_updates > 0,
            "Telegram max concurrent updates must be positive"
        );
        anyhow::ensure!(
            !settings.file_transfer_timeout.is_zero(),
            "Telegram file transfer timeout must be positive"
        );
        anyhow::ensure!(
            settings.max_concurrent_file_transfers > 0,
            "Telegram max concurrent file transfers must be positive"
        );
        anyhow::ensure!(
            settings.max_concurrent_updates <= MAX_CONCURRENT_UPDATES,
            "Telegram max concurrent updates is too large"
        );
        anyhow::ensure!(
            settings.polling_timeout.as_millis() >= 1000,
            "Telegram polling timeout must be at least one second"
        );
        anyhow::ensure!(
            settings.polling_timeout.subsec_nanos() == 0,
            "Telegram polling timeout must use whole seconds"
        );
        anyhow::ensure!(
            settings.polling_timeout.as_secs() <= u64::from(u32::MAX),
            "Telegram polling timeout is too large"
        );
        let burst = NonZeroU32::new(settings.rate_limit_burst)
            .context("Telegram rate limit burst must be positive")?;
        let quota = Quota::with_period(settings.rate_limit_replenish)
            .context("Telegram rate limit replenishment must be positive")?
            .allow_burst(burst);
        let polling_http_timeout = settings
            .polling_timeout
            .checked_add(settings.request_timeout)
            .context("Telegram polling HTTP timeout is too large")?;
        let api_url: reqwest::Url = api_base_url
            .parse()
            .with_context(|| format!("invalid Telegram API base URL: {api_base_url}"))?;
        let token = token.into();
        let bot = Bot::with_client(
            token.clone(),
            telegram_http_client(settings.request_timeout)?,
        )
        .set_api_url(api_url.clone());
        let polling_bot =
            Bot::with_client(token.clone(), telegram_http_client(polling_http_timeout)?)
                .set_api_url(api_url.clone());
        let transfer_bot =
            Bot::with_client(token, telegram_http_client(settings.file_transfer_timeout)?)
                .set_api_url(api_url);
        let admission = UpdateAdmission {
            rate_limiter: Arc::new(RateLimiter::keyed(quota)),
            semaphore: Arc::new(Semaphore::new(settings.max_concurrent_updates)),
            checks: Arc::new(AtomicU64::new(0)),
            rate_limit_rejections: Arc::new(AtomicU64::new(0)),
            concurrency_rejections: Arc::new(AtomicU64::new(0)),
            permits: Arc::new(Mutex::new(HashMap::new())),
        };

        Ok(Self {
            bot,
            polling_bot,
            application,
            settings,
            timezone,
            admission,
            file_transfers: FileTransfers::new(
                transfer_bot,
                settings.max_concurrent_file_transfers,
            ),
        })
    }

    pub async fn run(
        self,
        cancellation: CancellationToken,
        health: ComponentHealth,
    ) -> Result<(), TelegramBotError> {
        tokio::select! {
            () = cancellation.cancelled() => return Ok(()),
            result = self.bot.get_me().send() => {
                result.map_err(map_startup_error)?;
            }
        }
        let webhook = self
            .polling_bot
            .get_webhook_info()
            .send()
            .await
            .map_err(map_startup_error)?;
        if webhook.url.is_some() {
            self.polling_bot
                .delete_webhook()
                .send()
                .await
                .map_err(map_startup_error)?;
        }
        announce_commands(&self.bot).await;
        let _ready = health.ready();
        let storage = InMemStorage::<BotState>::new();
        let mut dispatcher = Dispatcher::builder(self.bot, schema())
            .dependencies(dptree::deps![
                storage,
                self.application,
                self.admission.clone(),
                self.file_transfers,
                self.timezone
            ])
            .distribution_function(|update| update.chat().map(|chat| chat.id))
            .worker_queue_size(self.settings.max_concurrent_updates)
            .default_handler(|_| async {})
            .error_handler(Arc::new(|error: anyhow::Error| async move {
                if let Some(teloxide::RequestError::RetryAfter(retry_after)) =
                    error.downcast_ref::<teloxide::RequestError>()
                {
                    tracing::warn!(
                        retry_after_seconds = retry_after.seconds(),
                        "Telegram rate limited an interactive request"
                    );
                } else {
                    tracing::error!(error = ?error, "Telegram update handler failed");
                }
            }))
            .build();
        let listener = Polling::builder(self.polling_bot)
            .timeout(self.settings.polling_timeout)
            .build();
        let invalid_credentials = Arc::new(AtomicBool::new(false));
        let listener = admitted_listener(
            listener,
            self.admission.clone(),
            invalid_credentials.clone(),
        );
        let shutdown = dispatcher.shutdown_token();
        let dispatch = dispatcher.try_dispatch_with_listener(
            listener,
            LoggingErrorHandler::with_custom_text("Telegram update listener failed"),
        );
        tokio::pin!(dispatch);

        tokio::select! {
            result = &mut dispatch => result.map_err(map_startup_error)?,
            () = cancellation.cancelled() => {
                let _ = shutdown.shutdown();
                dispatch.await.map_err(map_startup_error)?;
                return Ok(());
            }
        }

        if invalid_credentials.load(Ordering::Acquire) {
            Err(TelegramBotError::InvalidCredentials)
        } else {
            Err(TelegramBotError::UnexpectedTermination)
        }
    }
}

async fn announce_commands(bot: &Bot) {
    if let Err(error) = bot.set_my_commands(default_commands()).send().await {
        tracing::warn!(error = ?error, "failed to publish Telegram command hints");
    }
    if let Err(error) = bot
        .set_chat_menu_button()
        .menu_button(teloxide::types::MenuButton::Commands)
        .send()
        .await
    {
        tracing::warn!(error = ?error, "failed to set the Telegram menu button");
    }
}

fn map_startup_error(error: teloxide::RequestError) -> TelegramBotError {
    if matches!(
        error,
        teloxide::RequestError::Api(teloxide::ApiError::InvalidToken)
    ) {
        TelegramBotError::InvalidCredentials
    } else {
        TelegramBotError::Startup(error)
    }
}

fn telegram_http_client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(5)))
        .timeout(timeout)
        .tcp_nodelay(true)
        .build()
}

fn admit_update(update: &Update, admission: &UpdateAdmission) -> bool {
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
        log_rejection(
            &admission.rate_limit_rejections,
            telegram_user_id,
            update.id,
            "rate_limit",
        );
        return false;
    }

    match admission.semaphore.clone().try_acquire_owned() {
        Ok(permit) => {
            admission
                .permits
                .lock()
                .expect("Telegram admission permit mutex poisoned")
                .insert(update.id, permit);
            true
        }
        Err(_) => {
            log_rejection(
                &admission.concurrency_rejections,
                telegram_user_id,
                update.id,
                "concurrency_limit",
            );
            false
        }
    }
}

fn log_rejection(
    counter: &AtomicU64,
    telegram_user_id: TelegramUserId,
    update_id: UpdateId,
    kind: &'static str,
) {
    let rejected = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if rejected.is_power_of_two() {
        tracing::warn!(
            kind,
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
                    future::ready(match result {
                        Ok(update) if admit_update(&update, &admission) => Some(Ok(update)),
                        Ok(_) => None,
                        Err(error) => Some(Err(error)),
                    })
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

fn schema() -> UpdateHandler<anyhow::Error> {
    let messages = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(dptree::endpoint(handle_message));
    let callbacks = Update::filter_callback_query().endpoint(handle_callback);

    dptree::entry().filter_map(take_admission_permit).chain(
        dialogue::enter::<Update, InMemStorage<BotState>, BotState, _>()
            .branch(messages)
            .branch(callbacks),
    )
}
