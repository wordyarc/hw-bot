use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::{Actor, ApiError, AppState, JsonBody};
use crate::app::{RegisterStaff, RegisterStudent, RegisteredUser};
use crate::domain::{EducationLevel, StaffRole, StudentGroup, TelegramUserId};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterStudentRequest {
    display_name: String,
    student_group_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterStaffRequest {
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
    Actor(actor): Actor,
    JsonBody(request): JsonBody<RegisterStudentRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let command = RegisterStudent {
        telegram_user_id: actor,
        display_name: request.display_name,
        student_group_code: request.student_group_code,
        registered_at: Utc::now(),
    };

    let registered = state.users.register_student(command).await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&registered))))
}

pub async fn register_staff(
    State(state): State<AppState>,
    Actor(actor): Actor,
    JsonBody(request): JsonBody<RegisterStaffRequest>,
) -> Result<(StatusCode, Json<UserResponse>), ApiError> {
    let command = RegisterStaff {
        actor_telegram_user_id: actor,
        telegram_user_id: TelegramUserId::new(request.telegram_user_id)?,
        display_name: request.display_name,
        role: request.role,
        registered_at: Utc::now(),
    };

    let registered = state.users.register_staff(command).await?;

    Ok((StatusCode::CREATED, Json(UserResponse::from(&registered))))
}

/// Первый запрос бота на каждое сообщение: кто это и что ему показывать.
/// 404 здесь - сигнал «предложи регистрацию».
pub async fn me(
    State(state): State<AppState>,
    Actor(actor): Actor,
) -> Result<Json<UserResponse>, ApiError> {
    let registered = state.users.find_by_telegram_user_id(actor).await?;

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
