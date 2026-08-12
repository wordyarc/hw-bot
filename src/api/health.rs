use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::api::{AppState, JsonBody};

#[derive(Debug, Deserialize)]
pub struct EchoRequest {
    text: String,
}

#[derive(Debug, Serialize)]
pub struct EchoResponse {
    app: String,
    text: String,
}

pub async fn root() -> &'static str {
    "Hello, axum!"
}

pub async fn liveness() -> StatusCode {
    StatusCode::OK
}

pub async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    if !state.health.is_ready_without_database() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    match sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(1) => StatusCode::OK,
        Ok(_) => StatusCode::SERVICE_UNAVAILABLE,
        Err(error) => {
            crate::db::error::log_timeout(&error);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

pub async fn echo(
    State(state): State<AppState>,
    JsonBody(payload): JsonBody<EchoRequest>,
) -> Json<EchoResponse> {
    Json(EchoResponse {
        app: state.app_name.clone(),
        text: payload.text,
    })
}
