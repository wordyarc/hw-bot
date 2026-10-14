mod academic;
mod assignment;
mod attempts;
mod error;
mod files;
mod ids;
mod invites;
mod makeup;
mod submission;
mod text;
mod user;

#[cfg(test)]
mod tests;

pub use academic::{EducationLevel, StudentGroup, StudyProgram, Subject, TeachingCourse};
pub use assignment::{
    Assignment, AssignmentStatus, DeductedFrom, Grading, LATEST_TERM_YEAR, LatePenalty, LatePolicy,
    MAX_ATTEMPTS, MAX_LATE_PERIOD_DAYS, MAX_LATE_PERIODS, MAX_LATENESS_DAYS,
    MAX_PENALTY_PER_PERIOD, MAX_SCORE, MaxScore, NewAssignment,
};
pub use attempts::StudentAssignmentProgress;
pub use error::{DomainError, DomainResult};
pub use files::{
    FILE_NAME_MAX_LEN, FilePresentation, MIME_TYPE_MAX_LEN, StoredFileMetadata, StoredFileOwner,
};
pub use ids::{
    AssignmentId, StoredFileId, StudentGroupId, SubjectId, SubmissionId, TeachingCourseId,
    TelegramUserId, UserId,
};
pub use invites::InviteGeneration;
pub use makeup::{
    Acceptance, CourseMakeup, MAX_MAKEUP_ATTEMPTS, MAX_MAKEUP_DAYS, MakeupBalance, MakeupCredit,
    MakeupOffer, MakeupPool, MakeupRequest, MakeupTerms,
};
pub use submission::{
    Grade, Review, ReviewDecision, ReviewScore, Submission, SubmissionHistory, SubmissionKind,
    SubmissionStart, SubmissionStatus,
};
pub use text::{
    ASSIGNMENT_TITLE_MAX_LEN, DISPLAY_NAME_MAX_LEN, NonEmptyString, STUDENT_GROUP_CODE_MAX_LEN,
    SUBJECT_NAME_MAX_LEN, USER_TEXT_MAX_LEN,
};
pub use user::{Role, StaffRole, User};

pub(crate) use assignment::AssignmentParts;
pub(crate) use files::StoredFileParts;
pub(crate) use submission::{GradeParts, SubmissionHistoryParts, SubmissionParts};
