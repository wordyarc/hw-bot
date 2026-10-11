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
pub use courses::{
    ConfigureMakeup, CourseInviteState, CourseMakeupState, CourseService, MakeupDecision,
    MakeupQuestion, MakeupSettings, StudentContact, StudentGrades, StudentGradesCard,
};
pub use error::{AppError, AppResult, ErrorCode, Required};
pub use files::{
    AddAssignmentFile, AssignmentFiles, DeleteAssignmentFile, FilePolicy, FileService, OpenedFile,
};
pub use review::{
    ReviewQueueItem, ReviewSandbox, ReviewService, ReviewSubmission, ReviewedSubmission,
};
pub use submissions::{SubmissionService, SubmitHomework, SubmittedHomework};
pub use users::{CourseInvitation, RegisterStaff, RegisterStudent, RegisteredUser, UserService};

use crate::db::{DbError, DbResult};
pub use crate::read_models::{
    AssignmentGrade, AssignmentStudentSummary, CourseStudent, GradebookScore, MakeupPool,
    QueuePlace, SandboxSubmission, TeachingCourseSummary, UndecidedAssignment, UserStatistics,
    VersionedAssignment,
};

fn on_conflict<T>(result: DbResult<T>, conflict: AppError) -> AppResult<T> {
    match result {
        Err(DbError::Conflict { .. }) => Err(conflict),
        result => Ok(result?),
    }
}
