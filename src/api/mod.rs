use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::time::Duration;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

mod actor;
mod assignments;
mod courses;
mod error;
mod health;
mod json;
mod review;
mod state;
mod submissions;
mod users;

pub use actor::{ACTOR_HEADER, Actor, TelegramIdentity};
pub use error::ApiError;
pub use json::JsonBody;
pub use state::AppState;

pub const REQUEST_BODY_LIMIT: usize = 64 * 1024;

pub fn router(state: AppState, request_timeout: Duration) -> Router {
    Router::new()
        .route("/", get(health::root))
        .route("/health", get(health::health))
        .route("/echo", post(health::echo))
        .route(
            "/teaching-courses/{teaching_course_id}/assignments",
            get(assignments::list_course_assignments).post(assignments::create_assignment),
        )
        .route(
            "/assignments/{assignment_id}/publish",
            post(assignments::publish_assignment),
        )
        .route(
            "/assignments/{assignment_id}/archive",
            post(assignments::archive_assignment),
        )
        .route(
            "/assignments/{assignment_id}/deadline",
            post(assignments::change_deadline),
        )
        .route(
            "/assignments/{assignment_id}/students/{student_id}/extra-attempts",
            post(assignments::grant_extra_attempts),
        )
        .route(
            "/assignments/{assignment_id}/submissions",
            post(submissions::submit_homework),
        )
        .route("/student-groups", get(users::list_student_groups))
        .route("/students", post(users::register_student))
        .route("/staff", post(users::register_staff))
        .route("/me", get(users::me))
        .route(
            "/me/assignments",
            get(assignments::list_student_assignments),
        )
        .route(
            "/me/assignments/{assignment_id}",
            get(assignments::find_student_assignment),
        )
        .route("/me/courses", get(courses::list_my_courses))
        .route("/me/review-queue", get(review::queue_size))
        .route("/submissions/next", post(review::take_next_from_queue))
        .route(
            "/submissions/{submission_id}/review",
            post(review::review_submission),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            request_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::map_response(normalize_transport_error))
        .with_state(state)
}

async fn normalize_transport_error(response: Response) -> Response {
    match response.status() {
        StatusCode::REQUEST_TIMEOUT => ApiError::new(
            StatusCode::REQUEST_TIMEOUT,
            "request_timeout",
            "request timed out",
        )
        .into_response(),
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_body_too_large",
            "request body is too large",
        )
        .into_response(),
        _ => response,
    }
}
