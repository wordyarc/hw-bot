use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{ApiError, AppState, JsonBody};
use crate::app::SubmitHomework;
use crate::domain::{AssignmentId, Submission, SubmissionStatus, TelegramUserId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitHomeworkRequest {
    /// Identifies the student until authentication middleware takes over.
    telegram_user_id: i64,
    content: String,
}

#[derive(Debug, Serialize)]
pub struct SubmissionResponse {
    id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
    attempt_number: u16,
    content: String,
    status: &'static str,
    submitted_at: DateTime<Utc>,
}

impl From<&Submission> for SubmissionResponse {
    fn from(submission: &Submission) -> Self {
        Self {
            id: submission.id().as_uuid(),
            assignment_id: submission.assignment_id().as_uuid(),
            student_id: submission.student_id().as_uuid(),
            attempt_number: submission.attempt_number(),
            content: submission.content().as_str().to_owned(),
            status: status_name(submission.status()),
            submitted_at: submission.submitted_at(),
        }
    }
}

pub async fn submit_homework(
    State(state): State<AppState>,
    Path(assignment_id): Path<Uuid>,
    JsonBody(request): JsonBody<SubmitHomeworkRequest>,
) -> Result<(StatusCode, Json<SubmissionResponse>), ApiError> {
    let command = SubmitHomework {
        assignment_id: AssignmentId::from_uuid(assignment_id),
        telegram_user_id: TelegramUserId::new(request.telegram_user_id)?,
        content: request.content,
        submitted_at: Utc::now(),
    };

    let submission = state.submissions.submit_homework(command).await?;

    Ok((
        StatusCode::CREATED,
        Json(SubmissionResponse::from(&submission)),
    ))
}

fn status_name(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "pending_review",
        SubmissionStatus::Accepted => "accepted",
        SubmissionStatus::Rejected => "rejected",
    }
}
