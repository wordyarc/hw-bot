use chrono::{DateTime, Utc};

use crate::domain::{
    AssignmentId, StudentGroup, Subject, SubmissionHistory, TeachingCourse, User, UserId,
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
