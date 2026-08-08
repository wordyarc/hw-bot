use anyhow::{Context, ensure};
use hw_bot::api::{self, AppState};
use hw_bot::app::{NotificationWorker, NotificationWorkerSettings};
use hw_bot::bot::TelegramBot;
use hw_bot::config::{AppConfig, LogRotation, LoggingConfig, LoggingFormat};
use hw_bot::db;
use hw_bot::telegram::TelegramClient;
use std::net::SocketAddr;
use std::str::FromStr;
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
    tracing::info!(app = %config.app.name, "starting application");
    tracing::info!(
        max_connections = config.database.max_connections,
        run_migrations = config.database.run_migrations,
        "connecting to database"
    );
    let db_pool =
        db::connect_with_max_connections(&config.database.url, config.database.max_connections)
            .await
            .context("failed to connect to database")?;
    tracing::info!("connected to database");

    if config.database.run_migrations {
        tracing::info!("running database migrations");
        db::run_migrations(&db_pool)
            .await
            .context("failed to run database migrations")?;
        tracing::info!("database migrations completed");
    } else {
        tracing::info!("database migrations disabled");
    }

    let cancellation = CancellationToken::new();
    let telegram_bot = start_telegram_bot(&config, db_pool.clone(), cancellation.clone())?;
    let worker = start_notification_worker(&config, db_pool.clone(), cancellation.clone())?;
    let app = api::router(AppState::new(config.app.name, db_pool));

    let addr = SocketAddr::from_str(&format!("{}:{}", config.server.host, config.server.port))
        .context("failed to parse server bind address")?;
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

fn start_telegram_bot(
    config: &AppConfig,
    pool: sqlx::PgPool,
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
        pool,
        required_bot_token(config)?,
        &config.telegram.api_base_url,
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
