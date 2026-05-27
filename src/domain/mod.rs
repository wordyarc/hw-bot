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

#[cfg(test)]
mod id_tests {
    use crate::domain::{
        AssignmentId, ExtraAttemptGrantId, StudentGroupId, SubjectId, SubmissionId,
        TeachingCourseId, UserId,
    };

    #[test]
    fn write_heavy_entity_ids_are_uuid_v7() {
        assert_eq!(UserId::new().as_uuid().get_version_num(), 7);
        assert_eq!(TeachingCourseId::new().as_uuid().get_version_num(), 7);
        assert_eq!(AssignmentId::new().as_uuid().get_version_num(), 7);
        assert_eq!(SubmissionId::new().as_uuid().get_version_num(), 7);
        assert_eq!(ExtraAttemptGrantId::new().as_uuid().get_version_num(), 7);
    }

    #[test]
    fn low_write_dictionary_ids_remain_uuid_v4() {
        assert_eq!(StudentGroupId::new().as_uuid().get_version_num(), 4);
        assert_eq!(SubjectId::new().as_uuid().get_version_num(), 4);
    }
}

pub use academic::{EducationLevel, StudentGroup, StudyProgram, Subject, TeachingCourse};
pub use assignment::{Assignment, AssignmentStatus};
pub use attempts::ExtraAttemptGrant;
pub use error::{DomainError, DomainResult};
pub use ids::{
    AssignmentId, ExtraAttemptGrantId, StudentGroupId, SubjectId, SubmissionId, TeachingCourseId,
    TelegramUserId, UserId,
};
pub use submission::{Submission, SubmissionHistory, SubmissionStatus};
pub use text::NonEmptyString;
pub use user::{Role, User};
