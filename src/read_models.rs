use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, StoredFileMetadata, StudentGroup, Subject, SubmissionHistory,
    SubmissionId, TeachingCourse, User, UserId,
};

#[derive(Clone, Debug)]
pub struct TeachingCourseSummary {
    pub course: TeachingCourse,
    pub subject: Subject,
    pub group: StudentGroup,
}

#[derive(Clone, Debug)]
pub struct CourseStudent {
    pub student_id: UserId,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct AssignmentGrade {
    pub assignment_id: AssignmentId,
    pub title: String,
    pub max_score: u16,
    pub final_score: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct QueuePlace {
    pub assignment_id: AssignmentId,
    pub subject_name: String,
    pub assignment_title: String,
    pub submitted_at: DateTime<Utc>,
    pub position: u32,
    pub total: u32,
}

#[derive(Clone, Debug)]
pub struct SandboxSubmission {
    pub submission_id: SubmissionId,
    pub assignment_title: String,
    pub student_name: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AssignmentStudentSummary {
    pub student: User,
    pub history: SubmissionHistory,
}

#[derive(Clone, Debug)]
pub struct VersionedAssignment {
    pub assignment: Assignment,
    pub revision: i64,
}

#[derive(Clone, Debug)]
pub struct StoredFile {
    pub metadata: StoredFileMetadata,
    pub telegram_file_id: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct UserStatistics {
    pub assignments: u32,
    pub pending: u32,
    pub courses: u32,
}
