mod common;

use axum::http::StatusCode;
use common::TestApp;
use serde_json::{Value, json};

#[tokio::test]
async fn user_endpoints_cover_registration_and_reference_data() -> anyhow::Result<()> {
    let app = TestApp::start().await?;

    let group_code = app.fixture.group.code().as_str().to_owned();
    let admin = app.fixture.insert_admin(&app.pool, 1004).await?;
    let admin_id = admin.telegram_user_id().as_i64();
    let teacher_id = app.fixture.teacher.telegram_user_id().as_i64();

    let (status, body) = app.get("/student-groups").await?;
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

    let (status, body) = app.get("/users/2001").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["display_name"], json!("Ivan"));
    assert_eq!(body["student_group"]["code"], json!(group_code));

    let (status, body) = app.get("/users/9999").await?;
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

    let (status, body) = app
        .post(
            &format!(
                "/assignments/{}/submissions",
                app.fixture.assignment.id().as_uuid()
            ),
            json!({
                "telegram_user_id": 2001,
                "content": "https://github.com/course/hw/pull/1",
            }),
        )
        .await?;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a student registered over HTTP can submit right away"
    );
    assert_eq!(body["attempt_number"], json!(1));

    Ok(())
}

async fn register_student(
    app: &TestApp,
    telegram_user_id: i64,
    display_name: &str,
    student_group_code: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    app.post(
        "/students",
        json!({
            "telegram_user_id": telegram_user_id,
            "display_name": display_name,
            "student_group_code": student_group_code,
        }),
    )
    .await
}

async fn register_staff(
    app: &TestApp,
    actor_telegram_user_id: i64,
    telegram_user_id: i64,
    display_name: &str,
    role: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    app.post(
        "/staff",
        json!({
            "actor_telegram_user_id": actor_telegram_user_id,
            "telegram_user_id": telegram_user_id,
            "display_name": display_name,
            "role": role,
        }),
    )
    .await
}
