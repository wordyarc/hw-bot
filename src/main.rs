use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hw_bot::config::{AppConfig, LogRotation, LoggingConfig, LoggingFormat};
use hw_bot::db;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Clone)]
struct AppState {
    app_name: String,
    db_pool: sqlx::PgPool,
}

#[derive(Debug, Deserialize)]
struct EchoRequest {
    text: String,
}

#[derive(Debug, Serialize)]
struct EchoResponse {
    app: String,
    text: String,
}

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

    let state = Arc::new(AppState {
        app_name: config.app.name,
        db_pool,
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/echo", post(echo))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from_str(&format!("{}:{}", config.server.host, config.server.port))
        .context("failed to parse server bind address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind server listener to {addr}"))?;

    tracing::info!("listening on http://{addr}");

    axum::serve(listener, app)
        .await
        .context("failed to serve HTTP application")?;

    Ok(())
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
    match format {
        LoggingFormat::Compact => tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .compact()
                    .with_ansi(false)
                    .with_writer(writer),
            )
            .try_init()?,
        LoggingFormat::Pretty => tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .pretty()
                    .with_ansi(false)
                    .with_writer(writer),
            )
            .try_init()?,
        LoggingFormat::Json => tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_ansi(false)
                    .with_writer(writer),
            )
            .try_init()?,
    }

    Ok(())
}

async fn root() -> &'static str {
    "Hello, axum!"
}

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(1) => StatusCode::OK,
        Ok(_) | Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn echo(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<EchoRequest>,
) -> Json<EchoResponse> {
    Json(EchoResponse {
        app: state.app_name.clone(),
        text: payload.text,
    })
}
