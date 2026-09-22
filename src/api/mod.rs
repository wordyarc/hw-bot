use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

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
    Router::new()
        .route("/health", get(health::readiness))
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
            "/me/courses/{teaching_course_id}/gradebook",
            get(courses::course_gradebook),
        )
        .route("/me/review-queue", get(review::queue_size))
        .route("/submissions/next", post(review::take_next_from_queue))
        .route(
            "/submissions/{submission_id}/review",
            post(review::review_submission),
        )
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "not_found", "route not found") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method_not_allowed",
                "method not allowed",
            )
        })
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
    // Оборачиваем пустые ответы Tower. Сам по себе статус 413/408 не повод заменять готовое тело API.
    if response
        .extensions()
        .get::<error::ApiErrorResponse>()
        .is_some()
    {
        return response;
    }
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

#[cfg(test)]
mod tests {
    use axum::body::{Body, Bytes, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::app::AppError;

    #[tokio::test]
    async fn normalization_preserves_the_file_size_error() {
        let error = AppError::FileTooLarge {
            max_bytes: 5,
            actual_bytes: 6,
        };
        let message = error.to_string();
        let response = ApiError::from(error).into_response();

        let response = normalize_transport_error(response).await;

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "file_too_large");
        assert_eq!(body["error"]["message"], message);
    }

    #[tokio::test]
    async fn streamed_body_over_the_limit_keeps_the_transport_error_code() {
        let router = Router::new()
            .route(
                "/",
                post(|JsonBody(_): JsonBody<serde_json::Value>| async {}),
            )
            .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
            .layer(axum::middleware::map_response(normalize_transport_error));
        let chunks = futures::stream::iter([
            Ok::<_, std::io::Error>(Bytes::from_static(b"{\"text\":\"")),
            Ok(Bytes::from(vec![b'a'; REQUEST_BODY_LIMIT])),
            Ok(Bytes::from_static(b"\"}")),
        ]);
        let request = Request::post("/")
            .header("content-type", "application/json")
            .body(Body::from_stream(chunks))
            .unwrap();

        let response = router.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "request_body_too_large");
    }
}
