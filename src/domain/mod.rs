mod academic;
mod assignment;
mod attempts;
mod error;
mod ids;
mod submission;
mod text;
mod user;

#[cfg(test)]
mod tests;

pub use academic::{EducationLevel, StudentGroup, StudyProgram, Subject, TeachingCourse};
pub use assignment::{Assignment, AssignmentStatus, MAX_ATTEMPTS};
pub use attempts::StudentAssignmentProgress;
pub use error::{DomainError, DomainResult};
pub use ids::{
    AssignmentId, StudentGroupId, SubjectId, SubmissionId, TeachingCourseId, TelegramUserId, UserId,
};
pub use submission::{ReviewDecision, Submission, SubmissionHistory, SubmissionStatus};
pub use text::{
    ASSIGNMENT_TITLE_MAX_LEN, DISPLAY_NAME_MAX_LEN, NonEmptyString, STUDENT_GROUP_CODE_MAX_LEN,
    SUBJECT_NAME_MAX_LEN, USER_TEXT_MAX_LEN,
};
pub use user::{Role, StaffRole, User};

pub(crate) use assignment::AssignmentParts;
pub(crate) use submission::SubmissionParts;
