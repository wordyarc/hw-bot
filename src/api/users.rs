use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{ApiError, AppState, JsonBody};
use crate::app::{RegisterStaff, RegisterStudent, RegisteredUser};
use crate::domain::{EducationLevel, StaffRole, StudentGroup, TelegramUserId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterStudentRequest {
    telegram_user_id: i64,
    display_name: String,
    student_group_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterStaffRequest {
    /// Identifies the admin doing the registration until authentication middleware takes over.
    actor_telegram_user_id: i64,
    telegram_user_id: i64,
    display_name: String,
    role: StaffRole,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    id: Uuid,
    telegram_user_id: i64,
    display_name: String,
    role: &'static str,
    registered_at: DateTime<Utc>,
    student_group: Option<StudentGroupResponse>,
}

#[derive(Debug, Serialize)]
pub struct StudentGroupResponse {
    id: Uuid,
    code: String,
    education_level: EducationLevel,
    study_year: u8,
}

impl From<&RegisteredUser> for UserResponse {
    fn from(registered: &RegisteredUser) -> Self {
        Self {
            id: registered.user.id().as_uuid(),
            telegram_user_id: registered.user.telegram_user_id().as_i64(),
            display_name: registered.user.display_name().as_str().to_owned(),
            role: registered.user.role().as_str(),
            registered_at: registered.user.registered_at(),
            student_group: registered
                .student_group
                .as_ref()
                .map(StudentGroupResponse::from),
        }
    }
}

impl From<&StudentGroup> for StudentGroupResponse {
    fn from(group: &StudentGroup) -> Self {
        Self {
            id: group.id().as_uuid(),
            code: group.code().as_str().to_owned(),
            education_level: group.program().education_level(),
            study_year: group.program().year(),
        }
    }
}

pub async fn register_student(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<RegisterStudentRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let command = RegisterStudent {
        telegram_user_id: TelegramUserId::new(request.telegram_user_id)?,
        display_name: request.display_name,
        student_group_code: request.student_group_code,
        registered_at: Utc::now(),
    };

    let registered = state.users.register_student(command).await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&registered))))
}

pub async fn register_staff(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<RegisterStaffRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let command = RegisterStaff {
        actor_telegram_user_id: TelegramUserId::new(request.actor_telegram_user_id)?,
        telegram_user_id: TelegramUserId::new(request.telegram_user_id)?,
        display_name: request.display_name,
        role: request.role,
        registered_at: Utc::now(),
    };

    let registered = state.users.register_staff(command).await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&registered))))
}

pub async fn find_user(
    State(state): State<AppState>,
    Path(telegram_user_id): Path<i64>,
) -> Result<Json<UserResponse>, ApiError> {
    let telegram_user_id = TelegramUserId::new(telegram_user_id)?;
    let registered = state
        .users
        .find_by_telegram_user_id(telegram_user_id)
        .await?;

    Ok(Json(UserResponse::from(&registered)))
}

pub async fn list_student_groups(
    State(state): State<AppState>,
) -> Result<Json<Vec<StudentGroupResponse>>, ApiError> {
    let groups = state.users.list_student_groups().await?;

    Ok(Json(
        groups.iter().map(StudentGroupResponse::from).collect(),
    ))
}
