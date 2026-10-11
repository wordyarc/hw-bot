use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ApiPath as Path;
use crate::api::submissions::SubmissionResponse;
use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{ErrorCode, ReviewQueueItem, ReviewSubmission};
use crate::domain::{Assignment, ReviewDecision, ReviewScore, SubmissionId, User};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSubmissionRequest {
    decision: ReviewDecision,
    #[serde(default)]
    score: Option<u16>,
    #[serde(default)]
    max_score: Option<u16>,
    comment: String,
}

impl ReviewSubmissionRequest {
    fn score(&self) -> Result<Option<ReviewScore>, ApiError> {
        match (self.score, self.max_score) {
            (None, None) => Ok(None),
            (Some(value), Some(max_score)) => Ok(Some(ReviewScore { value, max_score })),
            _ => Err(ApiError::from_code(
                ErrorCode::InvalidInput,
                "score needs the max_score it was given against",
            )),
        }
    }
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
    max_score: Option<u16>,
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
            max_score: assignment.grading().max_score(),
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
    reviewer: Actor,
) -> Result<Json<ReviewQueueSizeResponse>, ApiError> {
    let pending = state.application.reviews.queue_size(reviewer).await?;

    Ok(Json(ReviewQueueSizeResponse { pending }))
}

pub async fn take_next_from_queue(
    State(state): State<AppState>,
    reviewer: Actor,
) -> Result<Response, ApiError> {
    let Some(item) = state
        .application
        .reviews
        .take_next_from_queue(reviewer)
        .await?
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };

    Ok(Json(ReviewQueueItemResponse::from(&item)).into_response())
}

pub async fn review_submission(
    State(state): State<AppState>,
    reviewer: Actor,
    Path(submission_id): Path<SubmissionId>,
    JsonBody(request): JsonBody<ReviewSubmissionRequest>,
) -> Result<Json<SubmissionResponse>, ApiError> {
    let command = ReviewSubmission {
        submission_id,
        actor: reviewer,
        decision: request.decision,
        score: request.score()?,
        comment: request.comment,
        reviewed_at: Utc::now(),
    };

    let reviewed = state.application.reviews.review_submission(command).await?;

    Ok(Json(SubmissionResponse::from(&reviewed.submission)))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn score(body: Value) -> Result<Option<ReviewScore>, ApiError> {
        serde_json::from_value::<ReviewSubmissionRequest>(body)
            .unwrap()
            .score()
    }

    #[test]
    fn a_score_travels_only_with_the_maximum_it_was_given_against() {
        assert_eq!(
            score(json!({"decision": "accept", "comment": "Ок"})).unwrap(),
            None
        );
        assert_eq!(
            score(json!({"decision": "accept", "score": 4, "max_score": 5, "comment": "Ок"}))
                .unwrap(),
            Some(ReviewScore {
                value: 4,
                max_score: 5
            })
        );
        for body in [
            json!({"decision": "accept", "score": 4, "comment": "Ок"}),
            json!({"decision": "accept", "max_score": 5, "comment": "Ок"}),
        ] {
            let error = score(body).unwrap_err();
            assert_eq!(
                (error.status(), error.code()),
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid_input")
            );
        }
    }
}
