use std::collections::BTreeSet;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ApiPath as Path;
use super::pagination::{Pagination, page_json};
use crate::api::submissions::SubmissionResponse;
use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{
    ChangeDeadline, ChangeGrading, CreateAssignment, ErrorCode, GrantExtraAttempts,
    StudentAssignment, StudentAssignmentDetails,
};
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DeductedFrom, FilePresentation, Grading,
    LatePolicy, MakeupCredit, MakeupTerms, StoredFileMetadata, StudentAssignmentProgress,
    StudentGroupId, TeachingCourseId, UserId,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateAssignmentRequest {
    title: String,
    body: String,
    #[serde(default)]
    student_group_ids: Option<Vec<StudentGroupId>>,
    max_attempts: u16,
    #[serde(default)]
    max_score: Option<u16>,
    #[serde(flatten)]
    late_policy: LatePolicyFields,
    deadline_at: DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct LatePolicyFields {
    #[serde(default)]
    late_period_days: Option<u16>,
    #[serde(default)]
    late_penalty_per_period: Option<u16>,
    #[serde(default)]
    late_max_periods: Option<u16>,
    #[serde(default)]
    late_deducted_from: Option<DeductedFrom>,
}

impl LatePolicyFields {
    fn parse(self) -> Result<Option<LatePolicy>, ApiError> {
        match (
            self.late_period_days,
            self.late_penalty_per_period,
            self.late_max_periods,
            self.late_deducted_from,
        ) {
            (None, None, None, None) => Ok(None),
            (
                Some(period_days),
                Some(penalty_per_period),
                Some(max_periods),
                Some(deducted_from),
            ) => Ok(Some(LatePolicy::new(
                period_days,
                penalty_per_period,
                max_periods,
                deducted_from,
            )?)),
            _ => Err(ApiError::from_code(
                ErrorCode::InvalidInput,
                "late policy needs late_period_days, late_penalty_per_period, late_max_periods and late_deducted_from together",
            )),
        }
    }
}

impl From<&Assignment> for LatePolicyFields {
    fn from(assignment: &Assignment) -> Self {
        assignment
            .late_policy()
            .map_or_else(Self::default, |policy| Self {
                late_period_days: Some(policy.period_days()),
                late_penalty_per_period: Some(policy.penalty_per_period()),
                late_max_periods: Some(policy.max_periods()),
                late_deducted_from: Some(policy.deducted_from()),
            })
    }
}

fn distinct_groups(ids: Vec<StudentGroupId>) -> Result<BTreeSet<StudentGroupId>, ApiError> {
    let count = ids.len();
    let groups: BTreeSet<_> = ids.into_iter().collect();
    if groups.len() != count {
        return Err(ApiError::from_code(
            ErrorCode::InvalidInput,
            "student_group_ids must not repeat a group",
        ));
    }

    Ok(groups)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeDeadlineRequest {
    deadline_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeGradingRequest {
    #[serde(default)]
    max_score: Option<u16>,
    #[serde(flatten)]
    late_policy: LatePolicyFields,
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
    student_group_ids: Vec<Uuid>,
    title: String,
    body: String,
    max_attempts: u16,
    max_score: Option<u16>,
    #[serde(flatten)]
    late_policy: LatePolicyFields,
    closes_at: DateTime<Utc>,
    makeup_offered: Option<bool>,
    makeup_credit: Option<u16>,
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
            student_group_ids: assignment
                .groups()
                .iter()
                .map(|group_id| group_id.as_uuid())
                .collect(),
            title: assignment.title().as_str().to_owned(),
            body: assignment.body().as_str().to_owned(),
            max_attempts: assignment.max_attempts(),
            max_score: assignment.grading().max_score(),
            late_policy: assignment.into(),
            closes_at: assignment.closes_at(),
            makeup_offered: assignment.makeup().map(MakeupTerms::is_offered),
            makeup_credit: assignment
                .makeup()
                .and_then(MakeupTerms::credit)
                .map(MakeupCredit::get),
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
    latest_submission: Option<SubmissionResponse>,
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
            subject: _,
            history,
            makeup: _,
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
            latest_submission: details.latest_submission.as_ref().map(Into::into),
        }
    }
}

pub async fn list_student_assignments(
    State(state): State<AppState>,
    actor: Actor,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<Vec<StudentAssignmentResponse>>), ApiError> {
    let assignments = state
        .application
        .assignments
        .list_for_student(actor, page)
        .await?;

    Ok(page_json(&assignments))
}

pub async fn list_archived_student_assignments(
    State(state): State<AppState>,
    actor: Actor,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<Vec<StudentAssignmentResponse>>), ApiError> {
    let assignments = state
        .application
        .assignments
        .list_archived_for_student(actor, page)
        .await?;

    Ok(page_json(&assignments))
}

pub async fn find_student_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<StudentAssignmentDetailsResponse>, ApiError> {
    let details = state
        .application
        .assignments
        .find_for_student(actor, assignment_id)
        .await?;

    Ok(Json(StudentAssignmentDetailsResponse::from(&details)))
}

pub async fn list_course_assignments(
    State(state): State<AppState>,
    actor: Actor,
    Path(teaching_course_id): Path<TeachingCourseId>,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<Vec<AssignmentResponse>>), ApiError> {
    let assignments = state
        .application
        .assignments
        .list_for_course(actor, teaching_course_id, page)
        .await?;

    Ok(page_json(&assignments))
}

pub async fn submission_history(
    State(state): State<AppState>,
    actor: Actor,
    Path(id): Path<AssignmentId>,
    Pagination(page): Pagination,
) -> Result<(HeaderMap, Json<Vec<SubmissionResponse>>), ApiError> {
    let submissions = state
        .application
        .assignments
        .submission_history(actor, id, page)
        .await?
        .submissions;
    Ok(page_json(&submissions))
}

pub async fn create_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(teaching_course_id): Path<TeachingCourseId>,
    JsonBody(request): JsonBody<CreateAssignmentRequest>,
) -> Result<(StatusCode, Json<AssignmentResponse>), ApiError> {
    let command = CreateAssignment {
        teaching_course_id,
        actor,
        title: request.title,
        body: request.body,
        student_group_ids: request.student_group_ids.map(distinct_groups).transpose()?,
        max_attempts: request.max_attempts,
        grading: Grading::from_max_score(request.max_score)?,
        late_policy: request.late_policy.parse()?,
        makeup: None,
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
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .application
        .assignments
        .publish(assignment_id, actor, Utc::now())
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn archive_assignment(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<AssignmentId>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let assignment = state
        .application
        .assignments
        .archive(assignment_id, actor)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn change_deadline(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<AssignmentId>,
    JsonBody(request): JsonBody<ChangeDeadlineRequest>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let command = ChangeDeadline {
        assignment_id,
        actor,
        deadline_at: request.deadline_at,
        expected_revision: None,
    };

    let assignment = state
        .application
        .assignments
        .change_deadline(command)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn change_grading(
    State(state): State<AppState>,
    actor: Actor,
    Path(assignment_id): Path<AssignmentId>,
    JsonBody(request): JsonBody<ChangeGradingRequest>,
) -> Result<Json<AssignmentResponse>, ApiError> {
    let command = ChangeGrading {
        assignment_id,
        actor,
        grading: Grading::from_max_score(request.max_score)?,
        late_policy: request.late_policy.parse()?,
        expected_revision: None,
    };

    let assignment = state
        .application
        .assignments
        .change_grading(command)
        .await?;

    Ok(Json(AssignmentResponse::from(&assignment)))
}

pub async fn grant_extra_attempts(
    State(state): State<AppState>,
    actor: Actor,
    Path((assignment_id, student_id)): Path<(AssignmentId, UserId)>,
    JsonBody(request): JsonBody<GrantExtraAttemptsRequest>,
) -> Result<Json<StudentAssignmentProgressResponse>, ApiError> {
    let command = GrantExtraAttempts {
        assignment_id,
        student_id,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn missing_and_null_policy_fields_mean_no_policy() {
        for value in [
            json!({}),
            json!({
                "late_period_days": null, "late_penalty_per_period": null, "late_max_periods": null,
                "late_deducted_from": null,
            }),
        ] {
            let request: LatePolicyFields = serde_json::from_value(value).unwrap();
            assert_eq!(request.parse().unwrap(), None);
        }
    }

    #[test]
    fn a_partial_policy_is_never_silently_ignored() {
        let fields = [
            ("late_period_days", json!(1)),
            ("late_penalty_per_period", json!(1)),
            ("late_max_periods", json!(1)),
            ("late_deducted_from", json!("course_total")),
        ];
        for mask in 1..15 {
            for missing_as_null in [false, true] {
                let mut value = json!({});
                for (index, (field, present)) in fields.iter().enumerate() {
                    if mask & (1 << index) != 0 {
                        value[field] = present.clone();
                    } else if missing_as_null {
                        value[field] = json!(null);
                    }
                }
                let request: LatePolicyFields = serde_json::from_value(value.clone()).unwrap();
                let error = request.parse().unwrap_err();
                assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY, "{value}");
                assert_eq!(error.code(), "invalid_input", "{value}");
            }
        }
    }

    #[test]
    fn a_complete_policy_preserves_each_independent_value() {
        let request: LatePolicyFields = serde_json::from_value(json!({
            "late_period_days": 3, "late_penalty_per_period": 2, "late_max_periods": 5,
            "late_deducted_from": "course_total",
        }))
        .unwrap();
        assert_eq!(
            request.parse().unwrap(),
            Some(LatePolicy::new(3, 2, 5, DeductedFrom::CourseTotal).unwrap())
        );
    }
}
