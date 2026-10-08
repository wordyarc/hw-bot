use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::ApiPath as Path;
use super::pagination::{Pagination, page_headers, page_json};
use crate::api::users::StudentGroupResponse;
use crate::api::{Actor, ApiError, AppState};
use crate::app::{StudentGrades, TeachingCourseSummary};
use crate::domain::{TeachingCourseId, UserId};
use crate::pagination::Page;

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
pub struct StudentGradesResponse {
    student_id: Uuid,
    display_name: String,
    work_total: u32,
    penalty_total: u32,
    total: i64,
    max_total: u32,
    grades: Vec<AssignmentGradeResponse>,
    grade_page: u32,
    next_grade_page: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AssignmentGradeResponse {
    assignment_id: Uuid,
    title: String,
    max_score: u16,
    work_score: Option<u16>,
    course_total_penalty: Option<u16>,
    makeup: Option<bool>,
}

impl From<&Page<StudentGrades>> for CourseGradebookResponse {
    fn from(gradebook: &Page<StudentGrades>) -> Self {
        Self {
            students: gradebook.items.iter().map(Into::into).collect(),
        }
    }
}

impl From<&StudentGrades> for StudentGradesResponse {
    fn from(student: &StudentGrades) -> Self {
        Self {
            student_id: student.student_id.as_uuid(),
            display_name: student.display_name.clone(),
            work_total: student.work_total(),
            penalty_total: student.penalty_total(),
            total: student.total(),
            max_total: student.max_total(),
            grade_page: student.grade_page,
            next_grade_page: student.next_grade_page,
            grades: student
                .grades
                .iter()
                .map(|grade| AssignmentGradeResponse {
                    assignment_id: grade.assignment_id.as_uuid(),
                    title: grade.title.clone(),
                    max_score: grade.max_score,
                    work_score: grade.score.map(|score| score.work_score),
                    course_total_penalty: grade.score.map(|score| score.course_total_penalty),
                    makeup: grade.score.map(|score| score.makeup),
                })
                .collect(),
        }
    }
}

pub async fn course_gradebook(
    State(state): State<AppState>,
    actor: Actor,
    Path(course_id): Path<TeachingCourseId>,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<CourseGradebookResponse>), ApiError> {
    let gradebook = state
        .application
        .courses
        .gradebook(actor, course_id, page)
        .await?;

    Ok((
        page_headers(&gradebook),
        Json(CourseGradebookResponse::from(&gradebook)),
    ))
}

pub async fn student_grades(
    State(state): State<AppState>,
    actor: Actor,
    Path((course, student)): Path<(TeachingCourseId, UserId)>,
    Pagination(page): Pagination,
) -> Result<Json<StudentGradesResponse>, ApiError> {
    let grades = state
        .application
        .courses
        .student_grades(actor, course, student, page)
        .await?;
    Ok(Json(StudentGradesResponse::from(&grades)))
}

pub async fn list_my_courses(
    State(state): State<AppState>,
    actor: Actor,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<Vec<TeachingCourseResponse>>), ApiError> {
    let courses = state
        .application
        .courses
        .list_for_teacher(actor, page)
        .await?;

    Ok(page_json(&courses))
}
