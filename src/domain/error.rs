use chrono::{DateTime, Utc};
use thiserror::Error;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },

    #[error("telegram user id must be positive, got {value}")]
    InvalidTelegramUserId { value: i64 },

    #[error("{education_level} study year must be in 1..={max}, got {value}")]
    InvalidStudyYear {
        education_level: &'static str,
        value: u8,
        max: u8,
    },

    #[error("attempt limit must be positive, got {value}")]
    InvalidAttemptLimit { value: u16 },

    #[error("attempt grant must be positive, got {value}")]
    InvalidAttemptGrant { value: u16 },

    #[error("assignment deadline must be after created_at")]
    AssignmentDeadlineMustBeAfterCreatedAt,

    #[error("assignment does not belong to teaching course")]
    AssignmentDoesNotBelongToTeachingCourse,

    #[error("teaching course has no teachers")]
    TeachingCourseHasNoTeachers,

    #[error("assignment is not published")]
    AssignmentIsNotPublished,

    #[error("assignment published_at does not match assignment status")]
    InvalidAssignmentPublishedState,

    #[error("assignment deadline exceeded: deadline_at {deadline_at}, submitted_at {submitted_at}")]
    AssignmentDeadlineExceeded {
        deadline_at: DateTime<Utc>,
        submitted_at: DateTime<Utc>,
    },

    #[error("attempts exhausted: allowed {allowed}, used {used}")]
    AttemptsExhausted { allowed: u16, used: u16 },

    #[error("submission attempt number must be positive, got {value}")]
    InvalidSubmissionAttemptNumber { value: u16 },

    #[error("submission review fields do not match submission status")]
    InvalidSubmissionReviewState,

    #[error("assignment already has an accepted submission")]
    SubmissionAlreadyAccepted,

    #[error("assignment already has a pending submission")]
    SubmissionAlreadyPendingReview,

    #[error("submission does not belong to assignment")]
    SubmissionDoesNotBelongToAssignment,

    #[error("submission is not pending review")]
    SubmissionIsNotPendingReview,

    #[error("student does not belong to assignment group")]
    StudentDoesNotBelongToAssignmentGroup,

    #[error("student assignment progress does not belong to assignment")]
    StudentProgressDoesNotBelongToAssignment,

    #[error("telegram user is already registered")]
    UserAlreadyRegistered,

    #[error("permission denied for {action}")]
    PermissionDenied { action: &'static str },
}
