use anyhow::Context;
use hw_bot::api::{self, AppState};
use hw_bot::app::{ApplicationContainer, FilePolicy};
use hw_bot::bot::{TelegramBot, TelegramBotSettings};
use hw_bot::config::{AppConfig, LogRotation, LoggingConfig, LoggingFormat, ValidatedAppConfig};
use hw_bot::db;
use hw_bot::health::{CriticalComponent, HealthState};
use hw_bot::supervision::{CriticalTask, Supervisor};
use hw_bot::telegram::{NotificationWorker, NotificationWorkerSettings, TelegramClient};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = AppConfig::load()
        .context("failed to load application config")?
        .validate()?;
    let _log_guard = init_logging(&config.logging)?;

    if let Err(error) = run(config).await {
        tracing::error!(error = ?error, "application failed");
        return Err(error);
    }

    Ok(())
}

async fn run(config: ValidatedAppConfig) -> anyhow::Result<()> {
    let file_policy = FilePolicy {
        max_file_size_bytes: usize::try_from(config.files.max_file_size_bytes)
            .expect("validated file size fits usize"),
        max_files_per_assignment: config.files.max_files_per_assignment,
        max_files_per_submission: config.files.max_files_per_submission,
    };
    tracing::info!(app = %config.app.name, "starting application");
    tracing::info!(
        max_connections = config.database.max_connections,
        run_migrations = config.database.run_migrations,
        run_bootstrap = config.database.run_bootstrap,
        "connecting to database"
    );
    let setup_pool =
        db::connect_with_max_connections(&config.database.url, config.database.max_connections)
            .await
            .context("failed to connect to database")?;
    tracing::info!("connected to database");

    if config.database.run_migrations {
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

    if config.database.run_bootstrap {
        let bootstrap_file = &config.database.bootstrap_file;
        tracing::info!(path = %bootstrap_file.display(), "running database bootstrap");
        let data = tokio::fs::read_to_string(bootstrap_file)
            .await
            .with_context(|| {
                format!("failed to read bootstrap file {}", bootstrap_file.display())
            })?;
        let seed = serde_json::from_str(&data).with_context(|| {
            format!(
                "failed to parse bootstrap file {}",
                bootstrap_file.display()
            )
        })?;
        db::run_bootstrap(&setup_pool, &seed)
            .await
            .context("failed to run database bootstrap")?;
        tracing::info!("database bootstrap completed");
    } else {
        tracing::info!("database bootstrap disabled");
    }

    setup_pool.close().await;
    let db_pool = db::connect_runtime(
        &config.database.url,
        config.database.max_connections,
        Duration::from_millis(config.database.acquire_timeout_ms),
        Duration::from_millis(config.database.statement_timeout_ms),
    )
    .await
    .context("failed to initialize runtime database pool")?;

    let cancellation = CancellationToken::new();
    let health = HealthState::new(config.telegram.enabled, config.notification_worker.enabled);
    let application = ApplicationContainer::new(db_pool.clone(), file_policy);
    let telegram_bot = build_telegram_bot(&config, application.clone())?;
    let worker = build_notification_worker(&config, db_pool.clone())?;
    let app = api::router(
        AppState::new(
            config.app.name.clone(),
            db_pool.clone(),
            application,
            health.clone(),
        ),
        Duration::from_millis(config.server.request_timeout_ms),
    );

    let addr = SocketAddr::new(config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind server listener to {addr}"))?;

    tracing::info!("listening on http://{addr}");

    let shutdown_grace = Duration::from_millis(
        config
            .server
            .request_timeout_ms
            .max(config.telegram.shutdown_grace_ms)
            .max(config.notification_worker.shutdown_grace_ms),
    );
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

    let result = supervisor.run(shutdown_signal()).await;
    db_pool.close().await;
    result
}

fn build_telegram_bot(
    config: &AppConfig,
    application: ApplicationContainer,
) -> anyhow::Result<Option<TelegramBot>> {
    if !config.telegram.enabled {
        tracing::info!("Telegram bot disabled");
        return Ok(None);
    }

    let bot = TelegramBot::new(
        application,
        required_bot_token(config)?,
        &config.telegram.api_base_url,
        TelegramBotSettings {
            request_timeout: Duration::from_millis(config.telegram.request_timeout_ms),
            polling_timeout: Duration::from_millis(config.telegram.polling_timeout_ms),
            rate_limit_replenish: Duration::from_millis(config.telegram.rate_limit_replenish_ms),
            rate_limit_burst: config.telegram.rate_limit_burst,
            max_concurrent_updates: config.telegram.max_concurrent_updates,
            file_transfer_timeout: Duration::from_millis(config.files.transfer_timeout_ms),
            max_concurrent_file_transfers: config.files.max_concurrent_transfers,
        },
    )?;

    tracing::info!("Telegram bot enabled");
    Ok(Some(bot))
}

fn build_notification_worker(
    config: &AppConfig,
    pool: sqlx::PgPool,
) -> anyhow::Result<Option<NotificationWorker>> {
    if !config.notification_worker.enabled {
        tracing::info!("review notification worker disabled");
        return Ok(None);
    }

    let worker = &config.notification_worker;
    let bot_token = required_bot_token(config)?;

    let telegram = TelegramClient::new(
        &config.telegram.api_base_url,
        bot_token,
        Duration::from_millis(config.telegram.request_timeout_ms),
    )
    .context("failed to initialize Telegram client")?;
    let worker = NotificationWorker::new(
        pool,
        telegram,
        NotificationWorkerSettings {
            poll_interval: Duration::from_millis(worker.poll_interval_ms),
            lease_duration: Duration::from_millis(worker.lease_duration_ms),
            max_attempts: worker.max_attempts,
            retry_initial: Duration::from_millis(worker.retry_initial_ms),
            retry_max: Duration::from_millis(worker.retry_max_ms),
        },
    );

    tracing::info!("review notification worker enabled");
    Ok(Some(worker))
}

fn required_bot_token(config: &AppConfig) -> anyhow::Result<&str> {
    config
        .telegram
        .bot_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .context("Telegram bot token is required when a Telegram component is enabled")
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

fn init_logging(config: &LoggingConfig) -> anyhow::Result<Option<WorkerGuard>> {
    let env_filter = EnvFilter::try_new(&config.level)?;

    if config.file_enabled {
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
            .build(&config.file_dir)
            .with_context(|| format!("failed to initialize log appender in {}", config.file_dir))?;
        let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

        if config.also_stdout {
            init_logging_subscriber(env_filter, config.format, std::io::stdout.and(file_writer))?;
        } else {
            init_logging_subscriber(env_filter, config.format, file_writer)?;
        }

        Ok(Some(guard))
    } else {
        init_logging_subscriber(env_filter, config.format, std::io::stdout)?;
        Ok(None)
    }
}

fn init_logging_subscriber<W>(
    env_filter: EnvFilter,
    format: LoggingFormat,
    writer: W,
) -> anyhow::Result<()>
where
    W: for<'writer> tracing_subscriber::fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer);

    let layer = match format {
        LoggingFormat::Compact => layer.compact().boxed(),
        LoggingFormat::Pretty => layer.pretty().boxed(),
        LoggingFormat::Json => layer.json().boxed(),
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(layer)
        .try_init()?;

    Ok(())
}
