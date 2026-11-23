mod access;
mod actor;
mod admin;
mod assignments;
mod container;
mod courses;
mod error;
mod files;
mod review;
mod submissions;
mod users;

pub use actor::Actor;
pub use admin::{
    AdminCourseCard, AdminService, ChangeStudentGroup, ClosePreview, ClosedCourse, CreateCourse,
    CreateStudentGroup, GroupChangeOffer, MIN_STUDENT_SEARCH_LEN, RegisterTeacher, StudentCard,
    TeacherCard,
};
pub use assignments::{
    AssignmentService, AssignmentStudents, AssignmentSubmissionHistory, ChangeDeadline,
    ChangeGrading, ChangeTerms, CreateAssignment, GrantExtraAttempts, StudentAssignment,
    StudentAssignmentDetails, TeacherAssignment,
};
pub use container::ApplicationContainer;
pub use courses::{
    ConfigureMakeup, CourseInviteState, CourseMakeupState, CoursePage, CourseService,
    GradebookEntry, MakeupDecision, MakeupQuestion, MakeupSettings, StudentContact, StudentGrades,
    StudentGradesCard,
};
pub use error::{AppError, AppResult, ErrorCode, Required};
pub use files::{
    AddAssignmentFile, AssignmentFiles, DeleteAssignmentFile, FilePolicy, FileService, OpenedFile,
};
pub use review::{
    RemainingAttempts, ReviewCard, ReviewCounters, ReviewSandbox, ReviewService, ReviewSubmission,
    ReviewedSubmission,
};
pub use submissions::{
    SubmissionService, SubmitHomework, SubmittedHomework, WithdrawSubmission, WithdrawalPreview,
};
pub use users::{
    CourseInvitation, RegisterStaff, RegisterStudent, RegisteredUser, RegistrationOffer,
    UserService,
};

use crate::db::{DbError, DbResult};
pub use crate::read_models::{
    AssignmentGrade, AssignmentStudentSummary, CourseRoster, CourseStudent, GradeSheet,
    GradeSheetCell, GradeSheetColumn, GradeSheetGroup, GradeSheetStudent, GradebookScore, LateMark,
    OfferedCourse, PendingSubmission, QueuePlace, RosterGroup, RosterStudent, ScoreCell,
    StoredSubject, StudentEnrollment, StudentListItem, SubjectQueue, TeachingCourseDetails,
    UndecidedAssignment, UserStatistics, VersionedAssignment,
};

fn on_conflict<T>(result: DbResult<T>, conflict: AppError) -> AppResult<T> {
    match result {
        Err(DbError::Conflict { .. }) => Err(conflict),
        result => Ok(result?),
    }
}
