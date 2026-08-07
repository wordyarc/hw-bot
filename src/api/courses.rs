use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::api::users::StudentGroupResponse;
use crate::api::{Actor, ApiError, AppState};
use crate::db::academic::TeachingCourseCard;

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

impl From<&TeachingCourseCard> for TeachingCourseResponse {
    fn from(card: &TeachingCourseCard) -> Self {
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

pub async fn list_my_courses(
    State(state): State<AppState>,
    Actor(actor): Actor,
) -> Result<Json<Vec<TeachingCourseResponse>>, ApiError> {
    let courses = state.courses.list_for_teacher(actor).await?;

    Ok(Json(courses.iter().map(Into::into).collect()))
}
