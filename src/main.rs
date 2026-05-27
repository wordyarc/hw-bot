use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    app_name: String,
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

    let state = Arc::new(AppState {
        app_name: "hw-bot".to_string(),
    });

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .route("/echo", post(echo))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!("listening on http://{addr}");

    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "Hello, axum!"
}

async fn health() -> impl IntoResponse {
    StatusCode::OK
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
