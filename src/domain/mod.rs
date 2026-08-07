mod academic;
mod assignment;
mod attempts;
mod enums;
mod error;
mod ids;
mod submission;
mod text;
mod user;

#[cfg(test)]
mod tests;

pub use academic::{EducationLevel, StudentGroup, StudyProgram, Subject, TeachingCourse};
pub use assignment::{Assignment, AssignmentStatus};
pub use attempts::StudentAssignmentProgress;
pub use error::{DomainError, DomainResult};
pub use ids::{
    AssignmentId, StudentGroupId, SubjectId, SubmissionId, TeachingCourseId, TelegramUserId, UserId,
};
pub use submission::{ReviewDecision, Submission, SubmissionHistory, SubmissionStatus};
pub use text::NonEmptyString;
pub use user::{Role, StaffRole, User};
