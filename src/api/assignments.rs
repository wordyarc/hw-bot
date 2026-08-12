use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::submissions::SubmissionResponse;
use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{
    ChangeDeadline, CreateAssignment, GrantExtraAttempts, StudentAssignment,
    StudentAssignmentDetails,
};
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, FilePresentation, StoredFileMetadata,
    StudentAssignmentProgress, TeachingCourseId, UserId,
};

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantExtraAttemptsRequest {
    amount: u16,
}

#[derive(Debug, Serialize)]
pub struct StudentAssignmentProgressResponse {
    assignment_id: Uuid,
    student_id: Uuid,
    extra_attempts: u16,
}

impl From<&StudentAssignmentProgress> for StudentAssignmentProgressResponse {
    fn from(progress: &StudentAssignmentProgress) -> Self {
        Self {
            assignment_id: progress.assignment_id().as_uuid(),
            student_id: progress.student_id().as_uuid(),
            extra_attempts: progress.extra_attempts(),
        }
    }
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

#[derive(Debug, Serialize)]
pub struct StudentAssignmentResponse {
    assignment: AssignmentResponse,
    progress: ProgressResponse,
}

/// Ответ даёт `Submission::submit`. Второго
/// источника не заводить, разойдутся.
#[derive(Debug, Serialize)]
struct ProgressResponse {
    used_attempts: u16,
    allowed_attempts: u16,
    remaining_attempts: u16,
    has_pending_submission: bool,
    has_accepted_submission: bool,
}

#[derive(Debug, Serialize)]
pub struct StudentAssignmentDetailsResponse {
    #[serde(flatten)]
    assignment: StudentAssignmentResponse,
    files: Vec<StoredFileResponse>,
    submissions: Vec<SubmissionResponse>,
}

#[derive(Debug, Serialize)]
struct StoredFileResponse {
    id: Uuid,
    position: u64,
    presentation: FilePresentation,
    filename: String,
    mime_type: Option<String>,
    size_bytes: u64,
}

impl From<&StoredFileMetadata> for StoredFileResponse {
    fn from(file: &StoredFileMetadata) -> Self {
        Self {
            id: file.id().as_uuid(),
            position: file.position(),
            presentation: file.presentation(),
            filename: file.filename().as_str().to_owned(),
            mime_type: file.mime_type().map(|value| value.as_str().to_owned()),
            size_bytes: file.size_bytes(),
        }
    }
}

impl From<&StudentAssignment> for StudentAssignmentResponse {
    fn from(view: &StudentAssignment) -> Self {
        Self {
            assignment: AssignmentResponse::from(&view.assignment),
            progress: ProgressResponse::from(view),
        }
    }
}

impl From<&StudentAssignment> for ProgressResponse {
    fn from(view: &StudentAssignment) -> Self {
        let StudentAssignment {
            assignment,
            history,
        } = view;

        Self {
            used_attempts: history.used_attempts(),
            allowed_attempts: history.allowed_attempts_for(assignment),
            remaining_attempts: history.remaining_attempts_for(assignment),
            has_pending_submission: history.has_pending_submission(),
            has_accepted_submission: history.has_accepted_submission(),
        }
    }
}

impl From<&StudentAssignmentDetails> for StudentAssignmentDetailsResponse {
    fn from(details: &StudentAssignmentDetails) -> Self {
        Self {
            assignment: StudentAssignmentResponse::from(&details.assignment),
            files: details.files.iter().map(Into::into).collect(),
            submissions: details.submissions.iter().map(Into::into).collect(),
        }
    }
}

pub async fn list_student_assignments(
    State(state): State<AppState>,
    actor: Actor,
) -> Result<Json<Vec<StudentAssignmentResponse>>, ApiError> {
    let assignments = state
        .application
        .assignments
        .list_for_student(actor)
        .await?;

    Ok(Json(assignments.iter().map(Into::into).collect()))
}

pub async fn find_student_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<StudentAssignmentDetailsResponse>, ApiError> {
    let details = state
        .application
        .assignments
        .find_for_student(actor, AssignmentId::from_uuid(assignment_id))
        .await?;

    Ok(Json(StudentAssignmentDetailsResponse::from(&details)))
}

pub async fn list_course_assignments(
    State(state): State<AppState>,
    actor: Actor,
    Path(teaching_course_id): Path<Uuid>,
) -> Result<Json<Vec<AssignmentResponse>>, ApiError> {
    let assignments = state
        .application
        .assignments
        .list_for_course(actor, TeachingCourseId::from_uuid(teaching_course_id))
        .await?;

    Ok(Json(assignments.iter().map(Into::into).collect()))
}

pub async fn create_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(teaching_course_id): Path<Uuid>,
    JsonBody(request): JsonBody<CreateAssignmentRequest>,
) -> Result<(StatusCode, Json<AssignmentResponse>), ApiError> {
    let command = CreateAssignment {
        teaching_course_id: TeachingCourseId::from_uuid(teaching_course_id),
        actor,
        title: request.title,
        body: request.body,
        max_attempts: request.max_attempts,
        created_at: Utc::now(),
        deadline_at: request.deadline_at,
    };

    let assignment = state.application.assignments.create(command).await?;

    Ok((
        StatusCode::CREATED,
        Json(AssignmentResponse::from(&assignment)),
    ))
}

pub async fn publish_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .application
        .assignments
        .publish(AssignmentId::from_uuid(assignment_id), actor, Utc::now())
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn archive_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .application
        .assignments
        .archive(AssignmentId::from_uuid(assignment_id), actor)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn change_deadline(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<Uuid>,
    JsonBody(request): JsonBody<ChangeDeadlineRequest>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let command = ChangeDeadline {
        assignment_id: AssignmentId::from_uuid(assignment_id),
        actor,
        deadline_at: request.deadline_at,
    };

    let assignment = state
        .application
        .assignments
        .change_deadline(command)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn grant_extra_attempts(
    State(state): State<AppState>,
    actor: Actor,
    Path((assignment_id, student_id)): Path<(Uuid, Uuid)>,
    JsonBody(request): JsonBody<GrantExtraAttemptsRequest>,
) -> Result<Json<StudentAssignmentProgressResponse>, ApiError> {
    let command = GrantExtraAttempts {
        assignment_id: AssignmentId::from_uuid(assignment_id),
        student_id: UserId::from_uuid(student_id),
        actor,
        amount: request.amount,
    };

    let progress = state
        .application
        .assignments
        .grant_extra_attempts(command)
        .await?;

    Ok(Json(StudentAssignmentProgressResponse::from(&progress)))
}
