use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::api::AppState;

pub async fn liveness() -> StatusCode {
    StatusCode::OK
}

pub async fn readiness(State(state): State<AppState>) -> impl IntoResponse {
    if !state.health.is_ready_without_database() {
        return StatusCode::SERVICE_UNAVAILABLE;
    }

    match sqlx::query!("select 1 as ready")
        .fetch_one(&state.db_pool)
        .await
    {
        Ok(_) => StatusCode::OK,
        Err(error) => {
            crate::db::error::log_timeout(&error);
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}
