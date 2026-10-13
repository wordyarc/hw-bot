use std::backtrace::{Backtrace, BacktraceStatus};
use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use hw_bot::api::{self, AppState};
use hw_bot::app::{ApplicationContainer, FilePolicy};
use hw_bot::bot::{BotTimezone, Clock, TelegramBot, TelegramBotSettings};
use hw_bot::config::{
    AppConfig, DatabaseConfig, LogRotation, LoggingConfig, LoggingFormat, ValidatedAppConfig,
};
use hw_bot::db;
use hw_bot::health::{CriticalComponent, HealthState};
use hw_bot::invites::CourseInviteCodec;
use hw_bot::supervision::{CriticalTask, Supervisor};
use hw_bot::telegram::{NotificationWorker, NotificationWorkerSettings, new_bot};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::writer::{BoxMakeWriter, MakeWriterExt};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load()
        .context("failed to load application config")?
        .validate()?;
    let _log_guard = init_logging(&config.logging)?;
    log_panics();

    if let Err(error) = run(config).await {
        tracing::error!(error = ?error, "application failed");
        return Err(error);
    }

    Ok(())
}

async fn run(config: ValidatedAppConfig) -> anyhow::Result<()> {
    let files = &config.files;
    let server = &config.server;
    tracing::info!(app = %config.app.name, "starting application");
    let db_pool = prepare_database(&config.database).await?;

    let file_policy = FilePolicy {
        max_file_size_bytes: files.max_file_size_bytes.get(),
        max_files_per_assignment: files.max_files_per_assignment.get(),
        max_files_per_submission: files.max_files_per_submission.get(),
    };
    let cancellation = CancellationToken::new();
    let health = HealthState::new(config.telegram.enabled, config.notification_worker.enabled);
    let application = ApplicationContainer::new(db_pool.clone(), file_policy);
    let invites = CourseInviteCodec::new(config.group_invites.invite_signing_secret.clone());
    let telegram_bot = build_telegram_bot(&config, application.clone(), invites.clone())?;
    let worker = build_notification_worker(&config, db_pool.clone())?;
    let app = api::router(
        AppState::new(db_pool.clone(), application, invites, health.clone()),
        ms(server.request_timeout_ms),
    );

    let addr = SocketAddr::new(server.host, server.port.get());
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind server listener to {addr}"))?;

    tracing::info!("listening on http://{addr}");

    let shutdown_grace = config.shutdown_grace();
    let mut supervisor = Supervisor::new(cancellation.clone(), health.clone(), shutdown_grace);

    let http_cancellation = cancellation.clone();
    supervisor.spawn(CriticalTask::Http, async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(http_cancellation.cancelled_owned())
            .await
            .context("failed to serve HTTP application")
    });

    if let Some(bot) = telegram_bot {
        let bot_cancellation = cancellation.clone();
        let component = health.component(CriticalComponent::TelegramBot);
        supervisor.spawn(CriticalTask::TelegramBot, async move {
            bot.run(bot_cancellation, component)
                .await
                .map_err(Into::into)
        });
    }

    if let Some(worker) = worker {
        let worker_cancellation = cancellation.clone();
        let component = health.component(CriticalComponent::NotificationWorker);
        supervisor.spawn(CriticalTask::NotificationWorker, async move {
            worker
                .run(worker_cancellation, component)
                .await
                .map_err(Into::into)
        });
    }

    supervisor
        .run_with_cleanup(shutdown_signal(), async {
            db_pool.close().await;
            Ok(())
        })
        .await
}

async fn prepare_database(database: &DatabaseConfig) -> anyhow::Result<PgPool> {
    tracing::info!(
        max_connections = database.max_connections.get(),
        run_migrations = database.run_migrations,
        run_bootstrap = database.run_bootstrap,
        "connecting to database"
    );
    let setup_pool =
        db::connect_with_max_connections(&database.url, database.max_connections.get())
            .await
            .context("failed to connect to database")?;
    tracing::info!("connected to database");

    if database.run_migrations {
        tracing::info!("running database migrations");
        db::run_migrations(&setup_pool)
            .await
            .context("failed to run database migrations")?;
        tracing::info!("database migrations completed");
    } else {
        tracing::info!("database migrations disabled, validating schema");
        db::validate_migrations(&setup_pool)
            .await
            .context("database schema does not match embedded migrations")?;
    }

    if database.run_bootstrap {
        bootstrap(&setup_pool, &database.bootstrap_file).await?;
    } else {
        tracing::info!("database bootstrap disabled");
    }

    setup_pool.close().await;
    db::connect_runtime(
        &database.url,
        database.max_connections.get(),
        ms(database.acquire_timeout_ms),
        ms(database.statement_timeout_ms),
    )
    .await
    .context("failed to initialize runtime database pool")
}

async fn bootstrap(pool: &PgPool, file: &Path) -> anyhow::Result<()> {
    tracing::info!(path = %file.display(), "running database bootstrap");
    let data = tokio::fs::read_to_string(file)
        .await
        .with_context(|| format!("failed to read bootstrap file {}", file.display()))?;
    let seed = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse bootstrap file {}", file.display()))?;
    db::run_bootstrap(pool, &seed)
        .await
        .context("failed to run database bootstrap")?;
    tracing::info!("database bootstrap completed");
    Ok(())
}

fn build_telegram_bot(
    config: &ValidatedAppConfig,
    application: ApplicationContainer,
    invites: CourseInviteCodec,
) -> anyhow::Result<Option<TelegramBot>> {
    let Some(token) = config.telegram_bot_token() else {
        tracing::info!("Telegram bot disabled");
        return Ok(None);
    };
    let telegram = &config.telegram;
    let files = &config.files;

    let bot = TelegramBot::new(
        application,
        token,
        &telegram.api_base_url,
        TelegramBotSettings {
            request_timeout: ms(telegram.request_timeout_ms),
            polling_timeout: ms(telegram.polling_timeout_ms),
            rate_limit_replenish: ms(telegram.rate_limit_replenish_ms),
            rate_limit_burst: telegram.rate_limit_burst,
            max_concurrent_updates: telegram.max_concurrent_updates,
            file_transfer_timeout: ms(files.transfer_timeout_ms),
            max_concurrent_file_transfers: files.max_concurrent_transfers,
        },
        BotTimezone::new(config.ui.timezone),
        Clock::system(),
        invites,
    )?;

    tracing::info!(timezone = %config.ui.timezone, "Telegram bot enabled");
    Ok(Some(bot))
}

fn build_notification_worker(
    config: &ValidatedAppConfig,
    pool: PgPool,
) -> anyhow::Result<Option<NotificationWorker>> {
    let Some(token) = config.notification_worker_token() else {
        tracing::info!("review notification worker disabled");
        return Ok(None);
    };
    let worker = &config.notification_worker;
    let request_timeout = ms(config.telegram.request_timeout_ms);

    let telegram = new_bot(token, &config.telegram.api_base_url, request_timeout)
        .context("failed to initialize Telegram client")?;
    let worker = NotificationWorker::new(
        pool,
        telegram,
        NotificationWorkerSettings {
            request_timeout,
            poll_interval: ms(worker.poll_interval_ms),
            lease_duration: ms(worker.lease_duration_ms),
            max_attempts: worker.max_attempts.get(),
            retry_initial: ms(worker.retry_initial_ms),
            retry_max: ms(worker.retry_max_ms),
        },
    );

    tracing::info!("review notification worker enabled");
    Ok(Some(worker))
}

fn ms(value: NonZeroU64) -> Duration {
    Duration::from_millis(value.get())
}

async fn shutdown_signal() -> anyhow::Result<()> {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .context("failed to install Ctrl+C handler")
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?
            .recv()
            .await;
        Ok::<_, std::io::Error>(())
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<std::io::Result<()>>();

    tokio::select! {
        result = ctrl_c => result?,
        result = terminate => result?,
    }

    Ok(())
}

/// Стандартный hook пишет в stderr простым текстом и в JSON-логи не попадает.
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("panic payload is not a string");
        let location = info.location().map(ToString::to_string);
        let backtrace = Backtrace::capture();
        let backtrace =
            (backtrace.status() == BacktraceStatus::Captured).then(|| backtrace.to_string());

        tracing::error!(
            panic.message = message,
            panic.location = location,
            panic.backtrace = backtrace,
            "panic"
        );
    }));
}

fn init_logging(config: &LoggingConfig) -> anyhow::Result<Option<WorkerGuard>> {
    let env_filter = EnvFilter::try_new(&config.level)?;

    let (writer, guard) = if config.file_enabled {
        std::fs::create_dir_all(&config.file_dir)
            .with_context(|| format!("failed to create log directory {}", config.file_dir))?;

        let file_appender = RollingFileAppender::builder()
            .rotation(match config.rotation {
                LogRotation::Daily => Rotation::DAILY,
                LogRotation::Hourly => Rotation::HOURLY,
                LogRotation::Never => Rotation::NEVER,
            })
            .filename_prefix(&config.file_prefix)
            .filename_suffix(&config.file_suffix)
            .max_log_files(config.max_files.get())
            .build(&config.file_dir)
            .with_context(|| format!("failed to initialize log appender in {}", config.file_dir))?;
        let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

        let writer = if config.also_stdout {
            BoxMakeWriter::new(std::io::stdout.and(file_writer))
        } else {
            BoxMakeWriter::new(file_writer)
        };

        (writer, Some(guard))
    } else {
        (BoxMakeWriter::new(std::io::stdout), None)
    };
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer);

    let layer = match config.format {
        LoggingFormat::Compact => layer.compact().boxed(),
        LoggingFormat::Pretty => layer.pretty().boxed(),
        LoggingFormat::Json => layer.json().boxed(),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .try_init()?;

    Ok(guard)
}
