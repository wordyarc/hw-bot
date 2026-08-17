use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::api::users::StudentGroupResponse;
use crate::api::{Actor, ApiError, AppState};
use crate::app::{CourseGradebook, StudentGrades, TeachingCourseSummary};
use crate::domain::TeachingCourseId;

#[derive(Debug, Serialize)]
pub struct TeachingCourseResponse {
    id: Uuid,
    subject: SubjectResponse,
    student_group: StudentGroupResponse,
    teacher_count: usize,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct SubjectResponse {
    id: Uuid,
    name: String,
}

impl From<&TeachingCourseSummary> for TeachingCourseResponse {
    fn from(card: &TeachingCourseSummary) -> Self {
        Self {
            id: card.course.id().as_uuid(),
            subject: SubjectResponse {
                id: card.subject.id().as_uuid(),
                name: card.subject.name().as_str().to_owned(),
            },
            student_group: StudentGroupResponse::from(&card.group),
            teacher_count: card.course.teacher_ids().len(),
            created_at: card.course.created_at(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CourseGradebookResponse {
    students: Vec<StudentGradesResponse>,
}

#[derive(Debug, Serialize)]
struct StudentGradesResponse {
    student_id: Uuid,
    display_name: String,
    total: u32,
    max_total: u32,
    grades: Vec<AssignmentGradeResponse>,
}

#[derive(Debug, Serialize)]
struct AssignmentGradeResponse {
    assignment_id: Uuid,
    title: String,
    max_score: u16,
    final_score: Option<u16>,
}

impl From<&CourseGradebook> for CourseGradebookResponse {
    fn from(gradebook: &CourseGradebook) -> Self {
        Self {
            students: gradebook.students.iter().map(Into::into).collect(),
        }
    }
}

impl From<&StudentGrades> for StudentGradesResponse {
    fn from(student: &StudentGrades) -> Self {
        Self {
            student_id: student.student_id.as_uuid(),
            display_name: student.display_name.clone(),
            total: student.total(),
            max_total: student.max_total(),
            grades: student
                .grades
                .iter()
                .map(|grade| AssignmentGradeResponse {
                    assignment_id: grade.assignment_id.as_uuid(),
                    title: grade.title.clone(),
                    max_score: grade.max_score,
                    final_score: grade.final_score,
                })
                .collect(),
        }
    }
}

pub async fn course_gradebook(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<Uuid>,
) -> Result<Json<CourseGradebookResponse>, ApiError> {
    let gradebook = state
        .application
        .courses
        .gradebook(actor, TeachingCourseId::from_uuid(course_id))
        .await?;

    Ok(Json(CourseGradebookResponse::from(&gradebook)))
}

pub async fn list_my_courses(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<TeachingCourseResponse>>, ApiError> {
    let courses = state.application.courses.list_for_teacher(actor).await?;

    Ok(Json(courses.iter().map(Into::into).collect()))
}
