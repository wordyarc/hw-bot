use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::app::AppError;
use crate::domain::DomainError;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    fn internal(error: &dyn std::fmt::Display) -> Self {
        tracing::error!(error = %error, "request failed");

        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "internal server error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            error: ApiErrorDetails {
                code: self.code,
                message: self.message,
            },
        };

        (self.status, Json(body)).into_response()
    }
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: ApiErrorDetails,
}

#[derive(Debug, Serialize)]
struct ApiErrorDetails {
    code: &'static str,
    message: String,
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Domain(domain_error) => Self::from(domain_error),
            AppError::NotFound { resource } => Self::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{resource} not found"),
            ),
            AppError::Conflict { message } => Self::new(StatusCode::CONFLICT, "conflict", message),
            AppError::AssignmentChangedConcurrently => Self::new(
                StatusCode::CONFLICT,
                "assignment_changed_concurrently",
                "assignment was changed by a concurrent request",
            ),
            AppError::Db(database_error) => Self::internal(&database_error),
        }
    }
}

impl From<DomainError> for ApiError {
    fn from(error: DomainError) -> Self {
        let (status, code) = match error {
            // Malformed input.
            DomainError::EmptyField { .. } => (StatusCode::UNPROCESSABLE_ENTITY, "empty_field"),
            DomainError::InvalidTelegramUserId { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_telegram_user_id")
            }
            DomainError::InvalidStudyYear { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_study_year")
            }
            DomainError::InvalidAttemptLimit { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_attempt_limit")
            }
            DomainError::InvalidAttemptGrant { .. } => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_attempt_grant")
            }
            DomainError::AssignmentDeadlineMustBeAfterCreatedAt => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_assignment_deadline",
            ),

            DomainError::AssignmentDoesNotBelongToTeachingCourse => (
                StatusCode::CONFLICT,
                "assignment_does_not_belong_to_teaching_course",
            ),
            DomainError::AssignmentIsNotPublished => {
                (StatusCode::CONFLICT, "assignment_is_not_published")
            }
            DomainError::AssignmentStatusTransitionNotAllowed { .. } => (
                StatusCode::CONFLICT,
                "assignment_status_transition_not_allowed",
            ),
            DomainError::AssignmentIsArchived => (StatusCode::CONFLICT, "assignment_is_archived"),
            DomainError::AssignmentDeadlineExceeded { .. } => {
                (StatusCode::CONFLICT, "assignment_deadline_exceeded")
            }
            DomainError::AttemptsExhausted { .. } => (StatusCode::CONFLICT, "attempts_exhausted"),
            DomainError::SubmissionAlreadyAccepted => {
                (StatusCode::CONFLICT, "submission_already_accepted")
            }
            DomainError::SubmissionAlreadyPendingReview => {
                (StatusCode::CONFLICT, "submission_already_pending_review")
            }
            DomainError::SubmissionDoesNotBelongToAssignment => (
                StatusCode::CONFLICT,
                "submission_does_not_belong_to_assignment",
            ),
            DomainError::SubmissionIsNotPendingReview => {
                (StatusCode::CONFLICT, "submission_is_not_pending_review")
            }
            DomainError::UserAlreadyRegistered => (StatusCode::CONFLICT, "user_already_registered"),
            DomainError::StudentProgressDoesNotBelongToAssignment => (
                StatusCode::CONFLICT,
                "student_progress_does_not_belong_to_assignment",
            ),

            DomainError::StudentDoesNotBelongToAssignmentGroup => (
                StatusCode::FORBIDDEN,
                "student_does_not_belong_to_assignment_group",
            ),
            DomainError::PermissionDenied { .. } => (StatusCode::FORBIDDEN, "permission_denied"),

            DomainError::InvalidAssignmentPublishedState
            | DomainError::InvalidSubmissionAttemptNumber { .. }
            | DomainError::InvalidSubmissionReviewState
            | DomainError::TeachingCourseHasNoTeachers => return Self::internal(&error),
        };

        Self::new(status, code, error.to_string())
    }
}

impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        Self::new(
            rejection.status(),
            "invalid_request_body",
            rejection.body_text(),
        )
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::db::DbError;

    fn api_error(error: DomainError) -> ApiError {
        ApiError::from(error)
    }

    #[test]
    fn maps_state_conflicts_to_conflict() {
        let error = api_error(DomainError::AttemptsExhausted {
            allowed: 2,
            used: 2,
        });

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), "attempts_exhausted");
        assert_eq!(error.message, "attempts exhausted: allowed 2, used 2");
    }

    #[test]
    fn maps_missed_deadline_to_conflict() {
        let error = api_error(DomainError::AssignmentDeadlineExceeded {
            deadline_at: Utc::now(),
            submitted_at: Utc::now(),
        });

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), "assignment_deadline_exceeded");
    }

    #[test]
    fn maps_authorization_failures_to_forbidden() {
        assert_eq!(
            api_error(DomainError::StudentDoesNotBelongToAssignmentGroup).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            api_error(DomainError::PermissionDenied {
                action: "submit homework"
            })
            .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn maps_malformed_input_to_unprocessable_entity() {
        let error = api_error(DomainError::EmptyField {
            field: "submission.content",
        });

        assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code(), "empty_field");
    }

    #[test]
    fn hides_corrupted_storage_invariants_behind_internal_error() {
        let error = api_error(DomainError::InvalidSubmissionReviewState);

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code(), "internal_error");
        assert_eq!(error.message, "internal server error");
    }

    #[test]
    fn maps_missing_resources_to_not_found() {
        let error = ApiError::from(AppError::NotFound {
            resource: "assignment",
        });

        assert_eq!(error.status(), StatusCode::NOT_FOUND);
        assert_eq!(error.code(), "not_found");
        assert_eq!(error.message, "assignment not found");
    }

    #[test]
    fn maps_assignment_write_races_to_a_specific_conflict() {
        let error = ApiError::from(AppError::AssignmentChangedConcurrently);

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), "assignment_changed_concurrently");
    }

    #[test]
    fn a_lost_race_reports_the_same_error_as_the_up_front_check() {
        for rule in [
            DomainError::SubmissionAlreadyPendingReview,
            DomainError::SubmissionAlreadyAccepted,
            DomainError::SubmissionIsNotPendingReview,
        ] {
            let from_check = ApiError::from(AppError::from(rule.clone()));
            let from_race = ApiError::from(AppError::from(DbError::RuleViolation(rule)));

            assert_eq!(from_check.status(), from_race.status());
            assert_eq!(from_check.code(), from_race.code());
            assert_eq!(from_check.message, from_race.message);
        }
    }
}
