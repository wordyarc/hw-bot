use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use hw_bot::config::AppConfig;
use hw_bot::db;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

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
    tracing_subscriber::fmt::init();

    let config = AppConfig::load()?;
    let db_pool =
        db::connect_with_max_connections(&config.database.url, config.database.max_connections)
            .await?;

    if config.database.run_migrations {
        db::run_migrations(&db_pool).await?;
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

    let addr = SocketAddr::from_str(&format!("{}:{}", config.server.host, config.server.port))?;
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("listening on http://{addr}");

    axum::serve(listener, app).await?;

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
