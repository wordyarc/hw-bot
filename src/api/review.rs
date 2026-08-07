use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::submissions::SubmissionResponse;
use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{ReviewQueueItem, ReviewSubmission};
use crate::domain::{Assignment, ReviewDecision, SubmissionId, User};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSubmissionRequest {
    decision: ReviewDecision,
    comment: String,
}

#[derive(Debug, Serialize)]
pub struct ReviewQueueItemResponse {
    submission: SubmissionResponse,
    assignment: QueuedAssignmentResponse,
    student: QueuedStudentResponse,
}

#[derive(Debug, Serialize)]
struct QueuedAssignmentResponse {
    id: Uuid,
    title: String,
    body: String,
    max_attempts: u16,
    deadline_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct QueuedStudentResponse {
    id: Uuid,
    telegram_user_id: i64,
    display_name: String,
}

impl From<&ReviewQueueItem> for ReviewQueueItemResponse {
    fn from(item: &ReviewQueueItem) -> Self {
        Self {
            submission: SubmissionResponse::from(&item.submission),
            assignment: QueuedAssignmentResponse::from(&item.assignment),
            student: QueuedStudentResponse::from(&item.student),
        }
    }
}

impl From<&Assignment> for QueuedAssignmentResponse {
    fn from(assignment: &Assignment) -> Self {
        Self {
            id: assignment.id().as_uuid(),
            title: assignment.title().as_str().to_owned(),
            body: assignment.body().as_str().to_owned(),
            max_attempts: assignment.max_attempts(),
            deadline_at: assignment.deadline_at(),
        }
    }
}

impl From<&User> for QueuedStudentResponse {
    fn from(student: &User) -> Self {
        Self {
            id: student.id().as_uuid(),
            telegram_user_id: student.telegram_user_id().as_i64(),
            display_name: student.display_name().as_str().to_owned(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ReviewQueueSizeResponse {
    pending: u32,
}

pub async fn queue_size(
    State(state): State<AppState>,
    Actor(reviewer): Actor,
) -> Result<Json<ReviewQueueSizeResponse>, ApiError> {
    let pending = state.reviews.queue_size(reviewer).await?;

    Ok(Json(ReviewQueueSizeResponse { pending }))
}

pub async fn take_next_from_queue(
    State(state): State<AppState>,
    Actor(reviewer): Actor,
) -> Result<Response, ApiError> {
    let Some(item) = state.reviews.take_next_from_queue(reviewer).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    Ok(Json(ReviewQueueItemResponse::from(&item)).into_response())
}

pub async fn review_submission(
    State(state): State<AppState>,
    Actor(reviewer): Actor,
    Path(submission_id): Path<Uuid>,
    JsonBody(request): JsonBody<ReviewSubmissionRequest>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    let command = ReviewSubmission {
        submission_id: SubmissionId::from_uuid(submission_id),
        reviewer_telegram_user_id: reviewer,
        decision: request.decision,
        comment: request.comment,
        reviewed_at: Utc::now(),
    };

    let submission = state.reviews.review_submission(command).await?;

    Ok(Json(SubmissionResponse::from(&submission)))
}
