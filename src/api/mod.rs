use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnFailure, TraceLayer};
use tracing::Level;

use crate::app::ErrorCode;

mod actor;
mod assignments;
mod courses;
mod error;
mod health;
mod json;
mod pagination;
mod path;
mod review;
mod state;
mod submissions;
mod users;

pub use actor::{ACTOR_HEADER, Actor, TelegramIdentity};
pub use error::ApiError;
pub use json::JsonBody;
pub use path::ApiPath;
pub use state::AppState;

pub const REQUEST_BODY_LIMIT: usize = 64 * 1024;

pub fn router(state: AppState, request_timeout: Duration) -> Router {
    let router = Router::new()
        .route("/health/live", get(health::liveness))
        .route("/health/ready", get(health::readiness))
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
            "/assignments/{assignment_id}/grading",
            post(assignments::change_grading),
        )
        .route(
            "/assignments/{assignment_id}/students/{student_id}/extra-attempts",
            post(assignments::grant_extra_attempts),
        )
        .route(
            "/assignments/{assignment_id}/submissions",
            post(submissions::submit_homework),
        )
        .route("/students", post(users::register_student))
        .route("/staff", post(users::register_staff))
        .route("/me", get(users::me))
        .route(
            "/me/assignments",
            get(assignments::list_student_assignments),
        )
        .route(
            "/me/assignments/archive",
            get(assignments::list_archived_student_assignments),
        )
        .route(
            "/me/assignments/{assignment_id}",
            get(assignments::find_student_assignment),
        )
        .route("/me/courses", get(courses::list_my_courses))
        .route(
            "/me/assignments/{assignment_id}/submissions",
            get(assignments::submission_history),
        )
        .route(
            "/me/courses/{course_id}/gradebook/{student_id}",
            get(courses::student_grades),
        )
        .route(
            "/me/courses/{course_id}/groups/{group_id}/gradebook",
            get(courses::course_gradebook),
        )
        .route("/me/review-queue", get(review::queue_size))
        .route("/submissions/next", post(review::take_next_from_queue))
        .route(
            "/submissions/{submission_id}/review",
            post(review::review_submission),
        )
        .fallback(|| async { ApiError::from_code(ErrorCode::NotFound, "route not found") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "method not allowed",
            )
        })
        .with_state(state);
    with_transport_layers(router, request_timeout)
}

fn with_transport_layers(router: Router, request_timeout: Duration) -> Router {
    router
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            request_timeout,
        ))
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
        .layer(axum::middleware::map_response(normalize_transport_error))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::WARN)),
        )
}

async fn normalize_transport_error(response: Response) -> Response {
    // Оборачиваем пустые ответы Tower. Сам по себе статус 413/408 не повод заменять готовое тело API.
    if response
        .extensions()
        .get::<error::ApiErrorResponse>()
        .is_some()
    {
        return response;
    }
    match response.status() {
        StatusCode::REQUEST_TIMEOUT => {
            tracing::warn!(kind = "request_timeout", "HTTP request timed out");
            ApiError::new(
                StatusCode::REQUEST_TIMEOUT,
                "request_timeout",
                "request timed out",
            )
            .into_response()
        }
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_body_too_large",
            "request body is too large",
        )
        .into_response(),
        _ => response,
    }
}

#[cfg(test)]
mod tests;
