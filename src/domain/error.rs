use std::error::Error;
use std::fmt::{Display, Formatter};

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainError {
    EmptyField {
        field: &'static str,
    },
    InvalidTelegramUserId {
        value: i64,
    },
    InvalidStudyYear {
        education_level: &'static str,
        value: u8,
        max: u8,
    },
    InvalidAttemptLimit {
        value: u16,
    },
    InvalidAttemptGrant {
        value: u16,
    },
    AssignmentDeadlineMustBeAfterCreatedAt,
    AssignmentDoesNotBelongToTeachingCourse,
    AssignmentIsNotPublished,
    AssignmentDeadlineExceeded {
        deadline_at: chrono::DateTime<chrono::Utc>,
        submitted_at: chrono::DateTime<chrono::Utc>,
    },
    AttemptsExhausted {
        allowed: u16,
        used: u16,
    },
    SubmissionAlreadyAccepted,
    SubmissionAlreadyPendingReview,
    SubmissionDoesNotBelongToAssignment,
    SubmissionIsNotPendingReview,
    StudentDoesNotBelongToAssignmentGroup,
    PermissionDenied {
        action: &'static str,
    },
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField { field } => write!(formatter, "{field} must not be empty"),
            Self::InvalidTelegramUserId { value } => {
                write!(formatter, "telegram user id must be positive, got {value}")
            }
            Self::InvalidStudyYear {
                education_level,
                value,
                max,
            } => write!(
                formatter,
                "{education_level} study year must be in 1..={max}, got {value}"
            ),
            Self::InvalidAttemptLimit { value } => {
                write!(formatter, "attempt limit must be positive, got {value}")
            }
            Self::InvalidAttemptGrant { value } => {
                write!(formatter, "attempt grant must be positive, got {value}")
            }
            Self::AssignmentDeadlineMustBeAfterCreatedAt => {
                write!(formatter, "assignment deadline must be after created_at")
            }
            Self::AssignmentDoesNotBelongToTeachingCourse => {
                write!(formatter, "assignment does not belong to teaching course")
            }
            Self::AssignmentIsNotPublished => write!(formatter, "assignment is not published"),
            Self::AssignmentDeadlineExceeded {
                deadline_at,
                submitted_at,
            } => write!(
                formatter,
                "assignment deadline exceeded: deadline_at {deadline_at}, submitted_at {submitted_at}"
            ),
            Self::AttemptsExhausted { allowed, used } => {
                write!(
                    formatter,
                    "attempts exhausted: allowed {allowed}, used {used}"
                )
            }
            Self::SubmissionAlreadyAccepted => {
                write!(formatter, "assignment already has an accepted submission")
            }
            Self::SubmissionAlreadyPendingReview => {
                write!(formatter, "assignment already has a pending submission")
            }
            Self::SubmissionDoesNotBelongToAssignment => {
                write!(formatter, "submission does not belong to assignment")
            }
            Self::SubmissionIsNotPendingReview => {
                write!(formatter, "submission is not pending review")
            }
            Self::StudentDoesNotBelongToAssignmentGroup => {
                write!(formatter, "student does not belong to assignment group")
            }
            Self::PermissionDenied { action } => {
                write!(formatter, "permission denied for {action}")
            }
        }
    }
}

impl Error for DomainError {}
