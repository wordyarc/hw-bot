use thiserror::Error;

use crate::db::DbError;
use crate::domain::DomainError;
use crate::string_enum::str_enum;

pub type AppResult<T> = Result<T, AppError>;

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ErrorCode {
        InternalError => "internal_error",
        NotFound => "not_found",
        InvalidInput => "invalid_input",
        PermissionDenied => "permission_denied",
        AssignmentUnavailable => "assignment_unavailable",
        DeadlineExceeded => "deadline_exceeded",
        SubmissionClosed => "submission_closed",
        AttemptsExhausted => "attempts_exhausted",
        SubmissionAlreadyAccepted => "submission_already_accepted",
        SubmissionPendingReview => "submission_pending_review",
        SubmissionAlreadyReviewed => "submission_already_reviewed",
        AlreadyRegistered => "already_registered",
        ConcurrentModification => "concurrent_modification",
        InviteInvalid => "invite_invalid",
        FileTooLarge => "file_too_large",
        FileLimitReached => "file_limit_reached",
        Overloaded => "overloaded"
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error("{resource} not found")]
    NotFound { resource: &'static str },

    #[error("assignment was changed by a concurrent request")]
    AssignmentChangedConcurrently,

    #[error("group invite was rotated by a concurrent request")]
    GroupInviteChangedConcurrently,

    #[error("group invite is not valid")]
    InviteInvalid,

    #[error("file must be at most {max_bytes} bytes, got {actual_bytes}")]
    FileTooLarge {
        max_bytes: usize,
        actual_bytes: usize,
    },

    #[error("assignment must have at most {max_files} files")]
    AssignmentFileLimitReached { max_files: usize },

    #[error("service is temporarily overloaded")]
    Overloaded,

    #[error(transparent)]
    Db(DbError),
}

impl AppError {
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Domain(error) => match error {
                DomainError::EmptyField { .. }
                | DomainError::FieldTooLong { .. }
                | DomainError::InvalidTelegramUserId { .. }
                | DomainError::InvalidStudyYear { .. }
                | DomainError::InvalidAttemptLimit { .. }
                | DomainError::InvalidAttemptGrant { .. }
                | DomainError::AttemptLimitExceeded { .. }
                | DomainError::InvalidMaxScore { .. }
                | DomainError::InvalidLatePolicy { .. }
                | DomainError::LatePolicyRequiresScoredGrading
                | DomainError::ScoreOutOfRange { .. }
                | DomainError::ScoreRequired
                | DomainError::ScoreNotAllowed
                | DomainError::AssignmentDeadlineMustBeAfterCreatedAt => ErrorCode::InvalidInput,
                DomainError::AssignmentIsNotPublished
                | DomainError::AssignmentStatusTransitionNotAllowed { .. }
                | DomainError::AssignmentIsArchived => ErrorCode::AssignmentUnavailable,
                DomainError::AssignmentDeadlineExceeded { .. } => ErrorCode::DeadlineExceeded,
                DomainError::SubmissionWindowClosed { .. } => ErrorCode::SubmissionClosed,
                DomainError::AttemptsExhausted { .. } => ErrorCode::AttemptsExhausted,
                DomainError::SubmissionAlreadyAccepted => ErrorCode::SubmissionAlreadyAccepted,
                DomainError::SubmissionAlreadyPendingReview => ErrorCode::SubmissionPendingReview,
                DomainError::SubmissionIsNotPendingReview => ErrorCode::SubmissionAlreadyReviewed,
                DomainError::StudentDoesNotBelongToAssignmentGroup
                | DomainError::PermissionDenied { .. } => ErrorCode::PermissionDenied,
                DomainError::UserAlreadyRegistered => ErrorCode::AlreadyRegistered,
                DomainError::InvalidInviteGeneration { .. }
                | DomainError::InviteGenerationExhausted
                | DomainError::AssignmentDoesNotBelongToTeachingCourse
                | DomainError::TeachingCourseHasNoTeachers
                | DomainError::InvalidAssignmentPublishedState
                | DomainError::InvalidSubmissionAttemptNumber { .. }
                | DomainError::InvalidSubmissionReviewState
                | DomainError::InvalidSubmissionGradeState
                | DomainError::SubmissionDoesNotBelongToAssignment
                | DomainError::StudentProgressDoesNotBelongToAssignment => ErrorCode::InternalError,
            },
            Self::NotFound { .. } => ErrorCode::NotFound,
            Self::AssignmentChangedConcurrently | Self::GroupInviteChangedConcurrently => {
                ErrorCode::ConcurrentModification
            }
            Self::InviteInvalid => ErrorCode::InviteInvalid,
            Self::FileTooLarge { .. } => ErrorCode::FileTooLarge,
            Self::AssignmentFileLimitReached { .. } => ErrorCode::FileLimitReached,
            Self::Overloaded => ErrorCode::Overloaded,
            Self::Db(_) => ErrorCode::InternalError,
        }
    }
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::RuleViolation(domain_error) => Self::Domain(domain_error),
            DbError::Overloaded => Self::Overloaded,
            other => Self::Db(other),
        }
    }
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::from(DbError::from(error))
    }
}

/// Превращает "нашлось или нет" в доменную ошибку слоя приложения, чтобы каждый use case не
/// повторял `.ok_or(AppError::NotFound { .. })`.
pub trait Required<T> {
    fn required(self, resource: &'static str) -> AppResult<T>;
}

impl<T> Required<T> for Option<T> {
    fn required(self, resource: &'static str) -> AppResult<T> {
        self.ok_or(AppError::NotFound { resource })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_rule_violations_keep_the_domain_code() {
        let direct = AppError::from(DomainError::SubmissionAlreadyPendingReview);
        let from_database = AppError::from(DbError::RuleViolation(
            DomainError::SubmissionAlreadyPendingReview,
        ));

        assert_eq!(direct.code(), ErrorCode::SubmissionPendingReview);
        assert_eq!(from_database.code(), direct.code());
    }

    #[test]
    fn validation_details_share_one_actionable_code() {
        let empty = AppError::from(DomainError::EmptyField { field: "title" });
        let too_long = AppError::from(DomainError::FieldTooLong {
            field: "title",
            max: 10,
            actual: 11,
        });

        assert_eq!(empty.code(), ErrorCode::InvalidInput);
        assert_eq!(too_long.code(), ErrorCode::InvalidInput);
    }

    #[test]
    fn unclassified_database_conflicts_are_internal() {
        let error = AppError::from(DbError::Conflict {
            message: "row changed",
        });

        assert_eq!(error.code(), ErrorCode::InternalError);
    }

    #[test]
    fn saturation_is_distinguishable_from_a_broken_request() {
        let error = AppError::from(DbError::Overloaded);

        assert_eq!(error.code(), ErrorCode::Overloaded);
    }

    #[test]
    fn corrupted_domain_state_is_internal() {
        let error = AppError::from(DomainError::InvalidSubmissionReviewState);

        assert_eq!(error.code(), ErrorCode::InternalError);
    }
}
