use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ApiPath as Path;
use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::SubmitHomework;
use crate::domain::{AssignmentId, Grade, Submission, SubmissionStatus, UserId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitHomeworkRequest {
    content: String,
}

#[derive(Debug, Serialize)]
pub struct SubmissionResponse {
    id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
    attempt_number: u16,
    content: String,
    status: SubmissionStatus,
    submitted_at: DateTime<Utc>,
    reviewed_by: Option<Uuid>,
    reviewed_at: Option<DateTime<Utc>>,
    review_comment: Option<String>,
    raw_score: Option<u16>,
    max_score: Option<u16>,
    late_periods: Option<u32>,
    penalty: Option<u16>,
    final_score: Option<u16>,
}

impl From<&Submission> for SubmissionResponse {
    fn from(submission: &Submission) -> Self {
        Self {
            id: submission.id().as_uuid(),
            assignment_id: submission.assignment_id().as_uuid(),
            student_id: submission.student_id().as_uuid(),
            attempt_number: submission.attempt_number(),
            content: submission.content().as_str().to_owned(),
            status: submission.status(),
            submitted_at: submission.submitted_at(),
            reviewed_by: submission.reviewed_by().map(UserId::as_uuid),
            reviewed_at: submission.reviewed_at(),
            review_comment: submission
                .review_comment()
                .map(|comment| comment.as_str().to_owned()),
            raw_score: submission.grade().map(Grade::raw_score),
            max_score: submission.grade().map(Grade::max_score),
            late_periods: submission.grade().map(Grade::late_periods),
            penalty: submission.grade().map(Grade::penalty),
            final_score: submission.grade().map(Grade::final_score),
        }
    }
}

pub async fn submit_homework(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<AssignmentId>,
    JsonBody(request): JsonBody<SubmitHomeworkRequest>,
) -> Result<(StatusCode, Json<SubmissionResponse>), ApiError> {
    let command = SubmitHomework {
        assignment_id,
        actor,
        content: request.content,
        submitted_at: Utc::now(),
    };

    let submission = state
        .application
        .submissions
        .submit_homework(command)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(SubmissionResponse::from(&submission)),
    ))
}
