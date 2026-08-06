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

pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query_scalar::<_, i32>("select 1")
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(1) => StatusCode::OK,
        Ok(_) | Err(_) => StatusCode::SERVICE_UNAVAILABLE,
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
