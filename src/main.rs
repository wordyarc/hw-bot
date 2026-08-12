use anyhow::{Context, ensure};
use hw_bot::api::{self, AppState};
use hw_bot::app::{ApplicationContainer, FilePolicy};
use hw_bot::bot::{TelegramBot, TelegramBotSettings};
use hw_bot::config::{AppConfig, FilesConfig, LogRotation, LoggingConfig, LoggingFormat};
use hw_bot::db;
use hw_bot::telegram::{NotificationWorker, NotificationWorkerSettings, TelegramClient};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::task::JoinHandle;
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
    let config = AppConfig::load().context("failed to load application config")?;
    let _log_guard = init_logging(&config.logging)?;

    if let Err(error) = run(config).await {
        tracing::error!(error = ?error, "application failed");
        return Err(error);
    }

    Ok(())
}

async fn run(config: AppConfig) -> anyhow::Result<()> {
    let file_policy = validate_files_config(&config.files)?;
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
        tracing::info!("database migrations disabled");
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
    ensure!(
        config.database.acquire_timeout_ms > 0,
        "database acquire timeout must be positive"
    );
    ensure!(
        config.database.statement_timeout_ms > 0,
        "database statement timeout must be positive"
    );
    let db_pool = db::connect_runtime(
        &config.database.url,
        config.database.max_connections,
        Duration::from_millis(config.database.acquire_timeout_ms),
        Duration::from_millis(config.database.statement_timeout_ms),
    )
    .await
    .context("failed to initialize runtime database pool")?;

    let cancellation = CancellationToken::new();
    let application = ApplicationContainer::new(db_pool.clone(), file_policy);
    let telegram_bot = start_telegram_bot(&config, application.clone(), cancellation.clone())?;
    let worker = start_notification_worker(&config, db_pool.clone(), cancellation.clone())?;
    ensure!(
        config.server.request_timeout_ms > 0,
        "HTTP request timeout must be positive"
    );
    let app = api::router(
        AppState::new(config.app.name, db_pool, application),
        Duration::from_millis(config.server.request_timeout_ms),
    );

    let addr = SocketAddr::new(config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind server listener to {addr}"))?;

    tracing::info!("listening on http://{addr}");

    let shutdown = cancellation.clone();
    let server_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown.cancel();
        })
        .await
        .context("failed to serve HTTP application");

    cancellation.cancel();
    stop_telegram_bot(
        telegram_bot,
        Duration::from_millis(config.telegram.shutdown_grace_ms),
    )
    .await?;
    stop_notification_worker(
        worker,
        Duration::from_millis(config.notification_worker.shutdown_grace_ms),
    )
    .await?;

    server_result
}

fn validate_files_config(config: &FilesConfig) -> anyhow::Result<FilePolicy> {
    const MAX_ASSIGNMENT_FILES: usize = 10;

    ensure!(
        config.max_file_size_bytes > 0,
        "file size limit must be positive"
    );
    ensure!(
        config.max_files_per_assignment > 0,
        "assignment file limit must be positive"
    );
    ensure!(
        config.max_files_per_assignment <= MAX_ASSIGNMENT_FILES,
        "assignment file limit must not exceed {MAX_ASSIGNMENT_FILES}"
    );
    ensure!(
        config.max_files_per_submission > 0,
        "submission file limit must be positive"
    );
    ensure!(
        config.transfer_timeout_ms > 0,
        "file transfer timeout must be positive"
    );
    ensure!(
        config.max_concurrent_transfers > 0,
        "max concurrent file transfers must be positive"
    );

    let max_file_size_bytes = usize::try_from(config.max_file_size_bytes)
        .context("file size limit does not fit this platform")?;
    max_file_size_bytes
        .checked_mul(config.max_concurrent_transfers)
        .context("total concurrent file transfer size is too large")?;

    Ok(FilePolicy {
        max_file_size_bytes,
        max_files_per_assignment: config.max_files_per_assignment,
        max_files_per_submission: config.max_files_per_submission,
    })
}

fn start_telegram_bot(
    config: &AppConfig,
    application: ApplicationContainer,
    cancellation: CancellationToken,
) -> anyhow::Result<Option<JoinHandle<()>>> {
    if !config.telegram.enabled {
        tracing::info!("Telegram bot disabled");
        return Ok(None);
    }

    ensure!(
        config.telegram.shutdown_grace_ms > 0,
        "Telegram bot shutdown grace must be positive"
    );
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
    Ok(Some(tokio::spawn(bot.run(cancellation))))
}

async fn stop_telegram_bot(bot: Option<JoinHandle<()>>, grace: Duration) -> anyhow::Result<()> {
    let Some(mut bot) = bot else {
        return Ok(());
    };

    match tokio::time::timeout(grace, &mut bot).await {
        Ok(result) => result.context("Telegram bot task failed"),
        Err(_) => {
            tracing::warn!("Telegram bot did not stop within shutdown grace");
            bot.abort();
            Ok(())
        }
    }
}

fn start_notification_worker(
    config: &AppConfig,
    pool: sqlx::PgPool,
    cancellation: CancellationToken,
) -> anyhow::Result<Option<JoinHandle<()>>> {
    if !config.notification_worker.enabled {
        tracing::info!("review notification worker disabled");
        return Ok(None);
    }

    let worker = &config.notification_worker;
    ensure!(
        worker.poll_interval_ms > 0,
        "worker poll interval must be positive"
    );
    ensure!(
        worker.lease_duration_ms > 0,
        "worker lease duration must be positive"
    );
    ensure!(
        worker.max_attempts > 0,
        "worker max attempts must be positive"
    );
    ensure!(
        worker.retry_initial_ms > 0,
        "worker initial retry must be positive"
    );
    ensure!(
        worker.retry_max_ms >= worker.retry_initial_ms,
        "worker max retry must not be less than initial retry"
    );
    ensure!(
        worker.shutdown_grace_ms > 0,
        "worker shutdown grace must be positive"
    );
    ensure!(
        config.telegram.request_timeout_ms > 0,
        "Telegram request timeout must be positive"
    );
    ensure!(
        worker.lease_duration_ms > config.telegram.request_timeout_ms,
        "worker lease duration must exceed Telegram request timeout"
    );

    let bot_token = required_bot_token(config)?;
    ensure!(
        !config.telegram.api_base_url.trim().is_empty(),
        "Telegram API base URL must not be empty"
    );

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
    Ok(Some(tokio::spawn(worker.run(cancellation))))
}

fn required_bot_token(config: &AppConfig) -> anyhow::Result<&str> {
    config
        .telegram
        .bot_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .context("Telegram bot token is required when a Telegram component is enabled")
}

async fn stop_notification_worker(
    worker: Option<JoinHandle<()>>,
    grace: Duration,
) -> anyhow::Result<()> {
    let Some(mut worker) = worker else {
        return Ok(());
    };

    match tokio::time::timeout(grace, &mut worker).await {
        Ok(result) => result.context("notification worker task failed"),
        Err(_) => {
            tracing::warn!("notification worker did not stop within shutdown grace");
            worker.abort();
            Ok(())
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

fn init_logging(config: &LoggingConfig) -> anyhow::Result<Option<WorkerGuard>> {
    let env_filter = EnvFilter::builder().parse_lossy(&config.level);

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

#[cfg(test)]
mod tests {
    use super::*;

    fn files_config() -> FilesConfig {
        FilesConfig {
            max_file_size_bytes: 5 * 1024 * 1024,
            max_files_per_assignment: 5,
            max_files_per_submission: 1,
            transfer_timeout_ms: 60_000,
            max_concurrent_transfers: 4,
        }
    }

    #[test]
    fn file_config_rejects_more_than_ten_assignment_files() {
        let error = validate_files_config(&FilesConfig {
            max_files_per_assignment: 11,
            ..files_config()
        })
        .unwrap_err();

        assert!(error.to_string().contains("must not exceed 10"));
    }

    #[test]
    fn file_config_rejects_total_transfer_size_overflow() {
        let error = validate_files_config(&FilesConfig {
            max_file_size_bytes: usize::MAX as u64,
            max_concurrent_transfers: 2,
            ..files_config()
        })
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("total concurrent file transfer size")
        );
    }
}
