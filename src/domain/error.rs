use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::domain::AssignmentStatus;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum DomainError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },

    #[error("{field} must not contain a null character")]
    NullCharacter { field: &'static str },

    #[error("{field} must be at most {max} characters, got {actual}")]
    FieldTooLong {
        field: &'static str,
        max: usize,
        actual: usize,
    },

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

    #[error("total attempt limit must not exceed {max}, got {value}")]
    AttemptLimitExceeded { value: u32, max: u16 },

    #[error("assignment max score must be in 1..={max}, got {value}")]
    InvalidMaxScore { value: u16, max: u16 },

    #[error("late policy {field} must be in 1..={max}, got {value}")]
    InvalidLatePolicy {
        field: &'static str,
        value: u16,
        max: u16,
    },

    #[error("late policy requires a scored assignment")]
    LatePolicyRequiresScoredGrading,

    #[error("assignment deadline must be after created_at")]
    AssignmentDeadlineMustBeAfterCreatedAt,

    #[error("computed assignment closing is outside the supported date range")]
    AssignmentClosingOutOfRange,

    #[error("late submission horizon must not exceed {max_days} days")]
    LateHorizonTooLong { max_days: u32 },

    #[error("assignment does not belong to teaching course")]
    AssignmentDoesNotBelongToTeachingCourse,

    #[error("teaching course has no teachers")]
    TeachingCourseHasNoTeachers,

    #[error("teaching course has no student groups")]
    TeachingCourseHasNoGroups,

    #[error("assignment must be issued to at least one student group")]
    AssignmentHasNoGroups,

    #[error("assignment can only be issued to student groups of its course")]
    AssignmentGroupOutsideCourse,

    #[error("assignment is not published")]
    AssignmentIsNotPublished,

    #[error("assignment cannot move from {} to {}", from.as_str(), to.as_str())]
    AssignmentStatusTransitionNotAllowed {
        from: AssignmentStatus,
        to: AssignmentStatus,
    },

    #[error("archived assignment cannot be changed")]
    AssignmentIsArchived,

    #[error("assignment published_at does not match assignment status")]
    InvalidAssignmentPublishedState,

    #[error("assignment deadline exceeded: deadline_at {deadline_at}, submitted_at {submitted_at}")]
    AssignmentDeadlineExceeded {
        deadline_at: DateTime<Utc>,
        submitted_at: DateTime<Utc>,
    },

    #[error("submissions are closed: closes_at {closes_at}, submitted_at {submitted_at}")]
    SubmissionWindowClosed {
        closes_at: DateTime<Utc>,
        submitted_at: DateTime<Utc>,
    },

    #[error("attempts exhausted: allowed {allowed}, used {used}")]
    AttemptsExhausted { allowed: u16, used: u16 },

    #[error("submission attempt number must be positive, got {value}")]
    InvalidSubmissionAttemptNumber { value: u16 },

    #[error("submission review fields do not match submission status")]
    InvalidSubmissionReviewState,

    #[error("submission grade fields do not match submission status")]
    InvalidSubmissionGradeState,

    #[error("score must be in 0..={max}, got {value}")]
    ScoreOutOfRange { value: u16, max: u16 },

    #[error("score was given out of {given_max}, but the assignment is graded out of {max}")]
    ScoreScaleMismatch { given_max: u16, max: u16 },

    #[error("scored assignment requires a score")]
    ScoreRequired,

    #[error("score is only allowed when accepting a scored assignment")]
    ScoreNotAllowed,

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

    #[error("makeup is not offered")]
    MakeupNotOffered,

    #[error("makeup terms do not fit the assignment grading")]
    MakeupTermsDoNotFitGrading,

    #[error("makeup credit must be in 1..={max}, got {value}")]
    InvalidMakeupCredit { value: u16, max: u16 },

    #[error("makeup attempts must be in 1..={max}, got {value}")]
    InvalidMakeupAttempts { value: u16, max: u16 },

    #[error("makeup must end in the future and at most {max_days} days ahead")]
    MakeupUntilOutOfRange { max_days: i64 },

    #[error("makeup attempts exhausted: allowed {allowed}, used {used}")]
    MakeupAttemptsExhausted { allowed: u16, used: u16 },

    #[error("at most {max} extra makeup attempts can be granted")]
    MakeupGrantLimitExceeded { max: u16 },

    #[error("stored course makeup settings are inconsistent")]
    InvalidCourseMakeupState,

    #[error("telegram user is already registered")]
    UserAlreadyRegistered,

    #[error("student group does not belong to the invited course")]
    StudentGroupNotInCourse,

    #[error("the invited course has several student groups, one must be chosen")]
    StudentGroupChoiceRequired,

    #[error("invite generation must be positive, got {value}")]
    InvalidInviteGeneration { value: u32 },

    #[error("invite generation cannot be rotated any further")]
    InviteGenerationExhausted,

    #[error("permission denied for {action}")]
    PermissionDenied { action: &'static str },
}
