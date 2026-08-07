use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{ChangeDeadline, CreateAssignment};
use crate::domain::{Assignment, AssignmentId, AssignmentStatus, TeachingCourseId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAssignmentRequest {
    title: String,
    body: String,
    max_attempts: u16,
    deadline_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeDeadlineRequest {
    deadline_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AssignmentResponse {
    id: Uuid,
    teaching_course_id: Uuid,
    subject_id: Uuid,
    student_group_id: Uuid,
    title: String,
    body: String,
    max_attempts: u16,
    status: AssignmentStatus,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

impl From<&Assignment> for AssignmentResponse {
    fn from(assignment: &Assignment) -> Self {
        Self {
            id: assignment.id().as_uuid(),
            teaching_course_id: assignment.teaching_course_id().as_uuid(),
            subject_id: assignment.subject_id().as_uuid(),
            student_group_id: assignment.student_group_id().as_uuid(),
            title: assignment.title().as_str().to_owned(),
            body: assignment.body().as_str().to_owned(),
            max_attempts: assignment.max_attempts(),
            status: assignment.status(),
            created_by: assignment.created_by().as_uuid(),
            created_at: assignment.created_at(),
            deadline_at: assignment.deadline_at(),
            published_at: assignment.published_at(),
        }
    }
}

pub async fn create_assignment(
    State(state): State<AppState>,
    Actor(actor): Actor,
    Path(teaching_course_id): Path<Uuid>,
    JsonBody(request): JsonBody<CreateAssignmentRequest>,
) -> Result<(StatusCode, Json<AssignmentResponse>), ApiError> {
    let command = CreateAssignment {
        teaching_course_id: TeachingCourseId::from_uuid(teaching_course_id),
        actor_telegram_user_id: actor,
        title: request.title,
        body: request.body,
        max_attempts: request.max_attempts,
        created_at: Utc::now(),
        deadline_at: request.deadline_at,
    };

    let assignment = state.assignments.create(command).await?;

    Ok((
        StatusCode::CREATED,
        Json(AssignmentResponse::from(&assignment)),
    ))
}

pub async fn publish_assignment(
    State(state): State<AppState>,
    Actor(actor): Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .assignments
        .publish(AssignmentId::from_uuid(assignment_id), actor, Utc::now())
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn archive_assignment(
    State(state): State<AppState>,
    Actor(actor): Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .assignments
        .archive(AssignmentId::from_uuid(assignment_id), actor)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn change_deadline(
    State(state): State<AppState>,
    Actor(actor): Actor,
    Path(assignment_id): Path<Uuid>,
    JsonBody(request): JsonBody<ChangeDeadlineRequest>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let command = ChangeDeadline {
        assignment_id: AssignmentId::from_uuid(assignment_id),
        actor_telegram_user_id: actor,
        deadline_at: request.deadline_at,
    };

    let assignment = state.assignments.change_deadline(command).await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}
