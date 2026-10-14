use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, Grading, StoredFileMetadata, StudentGroup, Subject,
    SubmissionHistory, SubmissionId, SubmissionKind, TeachingCourse, User, UserId,
};

#[derive(Clone, Debug)]
pub struct TeachingCourseSummary {
    pub course: TeachingCourse,
    pub subject: Subject,
    pub groups: Vec<StudentGroup>,
}

#[derive(Clone, Debug)]
pub struct CourseStudent {
    pub student_id: UserId,
    pub display_name: String,
}

#[derive(Clone, Debug)]
pub struct CourseRoster {
    pub subject: Subject,
    pub groups: Vec<RosterGroup>,
    pub page: u32,
    pub next_page: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct RosterGroup {
    pub code: String,
    pub size: u32,
    pub issued: u32,
    pub students: Vec<RosterStudent>,
}

#[derive(Clone, Debug)]
pub struct RosterStudent {
    pub position: u32,
    pub display_name: String,
    pub accepted: u32,
}

#[derive(Clone, Debug)]
pub struct AssignmentGrade {
    pub assignment_id: AssignmentId,
    pub title: String,
    pub max_score: u16,
    pub score: Option<GradebookScore>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GradebookScore {
    pub work_score: u16,
    pub course_total_penalty: u16,
    pub makeup: bool,
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
    pub kind: SubmissionKind,
    pub assignment_title: String,
    pub student_name: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AssignmentStudentSummary {
    pub student: User,
    pub group_code: String,
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

#[derive(Clone, Debug)]
pub struct UndecidedAssignment {
    pub assignment_id: AssignmentId,
    pub title: String,
    pub grading: Grading,
}
