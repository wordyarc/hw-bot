mod access;
mod actor;
mod assignments;
mod container;
mod courses;
mod error;
mod files;
mod review;
mod submissions;
mod users;

pub use actor::Actor;
pub use assignments::{
    AssignmentService, AssignmentSubmissionHistory, ChangeDeadline, ChangeGrading, ChangeTerms,
    CreateAssignment, GrantExtraAttempts, StudentAssignment, StudentAssignmentDetails,
};
pub use container::ApplicationContainer;
pub use courses::{CourseService, GroupInviteState, StudentGrades};
pub use error::{AppError, AppResult, ErrorCode, Required};
pub use files::{
    AddAssignmentFile, AssignmentFiles, DeleteAssignmentFile, FilePolicy, FileService, OpenedFile,
};
pub use review::{ReviewQueueItem, ReviewService, ReviewSubmission};
pub use submissions::{SubmissionService, SubmitHomework};
pub use users::{RegisterStaff, RegisterStudent, RegisteredUser, UserService, UserStatistics};

pub use crate::read_models::{
    AssignmentGrade, AssignmentStudentSummary, CourseStudent, QueuePlace, TeachingCourseSummary,
};

fn on_conflict<T>(result: crate::db::DbResult<T>, conflict: AppError) -> AppResult<T> {
    match result {
        Err(crate::db::DbError::Conflict { .. }) => Err(conflict),
        result => Ok(result?),
    }
}
