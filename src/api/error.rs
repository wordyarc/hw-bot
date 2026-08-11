use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::app::{AppError, ErrorCode};
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
            ErrorCode::InternalError.as_str(),
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
        let code = error.code();
        if code == ErrorCode::InternalError {
            return Self::internal(&error);
        }

        Self::new(app_error_status(code), code.as_str(), error.to_string())
    }
}

impl From<DomainError> for ApiError {
    fn from(error: DomainError) -> Self {
        Self::from(AppError::from(error))
    }
}

fn app_error_status(code: ErrorCode) -> StatusCode {
    match code {
        ErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::InvalidInput => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::AssignmentUnavailable
        | ErrorCode::DeadlineExceeded
        | ErrorCode::AttemptsExhausted
        | ErrorCode::SubmissionAlreadyAccepted
        | ErrorCode::SubmissionPendingReview
        | ErrorCode::SubmissionAlreadyReviewed
        | ErrorCode::AlreadyRegistered
        | ErrorCode::ConcurrentModification => StatusCode::CONFLICT,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
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
        assert_eq!(error.code(), "deadline_exceeded");
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
        assert_eq!(error.code(), "invalid_input");
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
        assert_eq!(error.code(), "concurrent_modification");
    }

    #[test]
    fn hides_unclassified_database_conflicts() {
        let error = ApiError::from(AppError::from(DbError::Conflict {
            message: "row changed",
        }));

        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code(), "internal_error");
        assert_eq!(error.message, "internal server error");
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
