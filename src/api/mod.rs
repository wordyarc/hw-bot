use axum::Router;
use axum::routing::{get, post};
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

pub fn router(state: AppState) -> Router {
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
