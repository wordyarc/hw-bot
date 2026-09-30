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
pub use courses::{CourseService, GroupInviteState, StudentContact, StudentGrades};
pub use error::{AppError, AppResult, ErrorCode, Required};
pub use files::{
    AddAssignmentFile, AssignmentFiles, DeleteAssignmentFile, FilePolicy, FileService,
};
pub use review::{ReviewQueueItem, ReviewService, ReviewSubmission};
pub use submissions::{SubmissionService, SubmitHomework};
pub use users::{RegisterStaff, RegisterStudent, RegisteredUser, UserService};

use crate::db::{DbError, DbResult};
pub use crate::read_models::{
    AssignmentGrade, AssignmentStudentSummary, CourseStudent, QueuePlace, StoredFile,
    TeachingCourseSummary, UserStatistics, VersionedAssignment,
};

fn on_conflict<T>(result: DbResult<T>, conflict: AppError) -> AppResult<T> {
    match result {
        Err(DbError::Conflict { .. }) => Err(conflict),
        result => Ok(result?),
    }
}
