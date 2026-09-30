use std::fmt;

use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, StoredFileMetadata, StudentGroup, Subject, SubmissionHistory,
    TeachingCourse, User, UserId,
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
pub struct AssignmentStudentSummary {
    pub student: User,
    pub history: SubmissionHistory,
}

#[derive(Clone, Debug)]
pub struct VersionedAssignment {
    pub assignment: Assignment,
    pub revision: i64,
}

pub struct StoredFile {
    pub metadata: StoredFileMetadata,
    pub content: Vec<u8>,
}

impl fmt::Debug for StoredFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredFile")
            .field("metadata", &self.metadata)
            .field("content_len", &self.content.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct UserStatistics {
    pub assignments: u32,
    pub pending: u32,
    pub courses: u32,
}
