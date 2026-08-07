mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Fixture, postgres_pool};
use hw_bot::api::{self, AppState};
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn user_endpoints_cover_registration_and_reference_data() -> anyhow::Result<()> {
    let (_postgres, pool) = postgres_pool().await?;
    hw_bot::db::run_migrations(&pool).await?;

    let fixture = Fixture::insert(&pool).await?;
    let app = api::router(AppState::new("hw-bot-test", pool.clone()));

    let group_code = fixture.group.code().as_str().to_owned();
    let admin = fixture.insert_admin(&pool, 1004).await?;
    let admin_id = admin.telegram_user_id().as_i64();
    let teacher_id = fixture.teacher.telegram_user_id().as_i64();

    let (status, body) = get(&app, "/student-groups").await?;
    assert_eq!(status, StatusCode::OK);
    let groups = body.as_array().unwrap();
    assert!(
        groups
            .iter()
            .any(|group| group["code"] == json!(group_code)),
        "the registration picker needs the group list"
    );
    assert_eq!(groups[0]["education_level"], json!("bachelor"));
    assert_eq!(groups[0]["study_year"], json!(1));

    let (status, body) = register_student(&app, 2001, "Ivan", &group_code).await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["role"], json!("student"));
    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["telegram_user_id"], json!(2001));
    assert_eq!(body["student_group"]["code"], json!(group_code));
    assert!(body["id"].as_str().is_some());

    let (status, body) = register_student(&app, 2001, "Ivan again", &group_code).await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "user_already_registered");

    let (status, body) = register_student(&app, 2002, "Petr", "NO-SUCH-GROUP").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "student group not found");

    let (status, body) = register_student(&app, 2002, "   ", &group_code).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "empty_field");

    let (status, body) = get(&app, "/users/2001").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["student_group"]["code"], json!(group_code));

    let (status, body) = get(&app, "/users/9999").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "user not found");

    let (status, body) = register_staff(&app, admin_id, 2003, "New teacher", "teacher").await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["role"], json!("teacher"));
    assert_eq!(
        body["student_group"],
        Value::Null,
        "staff is never bound to a student group"
    );

    let (status, body) = register_staff(&app, teacher_id, 2004, "Sneaky", "admin").await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"]["code"], "permission_denied",
        "an ordinary teacher cannot promote anyone"
    );

    let (status, body) = register_staff(&app, 2001, 2005, "Sneaky", "teacher").await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"]["code"], "permission_denied",
        "a student cannot make themselves staff"
    );

    let (status, body) = register_staff(&app, admin_id, 2006, "Not staff", "student").await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"]["code"], "invalid_request_body",
        "student is not a staff role, so it never reaches the domain"
    );

    let request = Request::post(format!(
        "/assignments/{}/submissions",
        fixture.assignment.id().as_uuid()
    ))
    .header("content-type", "application/json")
    .body(Body::from(serde_json::to_vec(&json!({
        "telegram_user_id": 2001,
        "content": "https://github.com/course/hw/pull/1",
    }))?))?;
    let (status, body) = send(&app, request).await?;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a student registered over HTTP can submit right away"
    );
    assert_eq!(body["attempt_number"], json!(1));

    Ok(())
}

async fn register_student(
    app: &Router,
    telegram_user_id: i64,
    display_name: &str,
    student_group_code: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post("/students")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "telegram_user_id": telegram_user_id,
            "display_name": display_name,
            "student_group_code": student_group_code,
        }))?))?;

    send(app, request).await
}

async fn register_staff(
    app: &Router,
    actor_telegram_user_id: i64,
    telegram_user_id: i64,
    display_name: &str,
    role: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post("/staff")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "actor_telegram_user_id": actor_telegram_user_id,
            "telegram_user_id": telegram_user_id,
            "display_name": display_name,
            "role": role,
        }))?))?;

    send(app, request).await
}

async fn get(app: &Router, path: &str) -> anyhow::Result<(StatusCode, Value)> {
    send(app, Request::get(path).body(Body::empty())?).await
}

async fn send(app: &Router, request: Request<Body>) -> anyhow::Result<(StatusCode, Value)> {
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;

    if bytes.is_empty() {
        return Ok((status, Value::Null));
    }

    Ok((status, serde_json::from_slice(&bytes)?))
}
