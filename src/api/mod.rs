use axum::Router;
use axum::routing::{get, post};
use tower_http::trace::TraceLayer;

mod error;
mod health;
mod json;
mod review;
mod state;
mod submissions;

pub use error::ApiError;
pub use json::JsonBody;
pub use state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(health::root))
        .route("/health", get(health::health))
        .route("/echo", post(health::echo))
        .route(
            "/assignments/{assignment_id}/submissions",
            post(submissions::submit_homework),
        )
        .route("/submissions/next", post(review::take_next_from_queue))
        .route(
            "/submissions/{submission_id}/review",
            post(review::review_submission),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
