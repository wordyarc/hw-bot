use crate::api::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

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
