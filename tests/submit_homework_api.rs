mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{Fixture, postgres_pool};
use hw_bot::api::{self, AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn submit_homework_endpoint_covers_the_full_request_lifecycle() -> anyhow::Result<()> {
    let (_postgres, pool) = postgres_pool().await?;
    hw_bot::db::run_migrations(&pool).await?;

    let fixture = Fixture::insert(&pool).await?;
    let app = api::router(AppState::new("hw-bot-test", pool.clone()));
    let assignment_id = fixture.assignment.id().as_uuid();
    let student_telegram_id = fixture.student.telegram_user_id().as_i64();

    let response = app
        .clone()
        .oneshot(Request::get("/health").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);

    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": student_telegram_id,
            "content": "  https://github.com/course/hw/pull/1  ",
        }),
    )
    .await?;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["assignment_id"], json!(assignment_id));
    assert_eq!(body["student_id"], json!(fixture.student.id().as_uuid()));
    assert_eq!(body["attempt_number"], json!(1));
    assert_eq!(body["status"], json!("pending_review"));
    assert_eq!(
        body["content"],
        json!("https://github.com/course/hw/pull/1"),
        "content is trimmed by the domain before it is stored"
    );
    assert!(body["id"].as_str().is_some());
    assert!(body["submitted_at"].as_str().is_some());

    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": student_telegram_id,
            "content": "https://github.com/course/hw/pull/2",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "submission_already_pending_review");

    let (status, body) = submit(
        &app,
        Uuid::now_v7(),
        json!({
            "telegram_user_id": student_telegram_id,
            "content": "https://github.com/course/hw/pull/3",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
    assert_eq!(body["error"]["message"], "assignment not found");

    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": 9999,
            "content": "https://github.com/course/hw/pull/4",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "student not found");

    let outsider = fixture.insert_outsider_student(&pool, 2001).await?;
    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": outsider.telegram_user_id().as_i64(),
            "content": "https://github.com/course/hw/pull/5",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"]["code"],
        "student_does_not_belong_to_assignment_group"
    );

    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": fixture.teacher.telegram_user_id().as_i64(),
            "content": "https://github.com/course/hw/pull/6",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");

    let draft = fixture.insert_draft_assignment(&pool).await?;
    let (status, body) = submit(
        &app,
        draft.id().as_uuid(),
        json!({
            "telegram_user_id": student_telegram_id,
            "content": "https://github.com/course/hw/pull/7",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"]["code"], "assignment_is_not_published");

    let second_student = fixture
        .insert_student(&pool, 1002, "Second student")
        .await?;
    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": second_student.telegram_user_id().as_i64(),
            "content": "   ",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "empty_field");

    let (status, body) = submit(
        &app,
        assignment_id,
        json!({
            "telegram_user_id": 0,
            "content": "https://github.com/course/hw/pull/8",
        }),
    )
    .await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_telegram_user_id");

    let (status, body) = submit(&app, assignment_id, json!({ "content": "no student" })).await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"]["code"], "invalid_request_body",
        "extractor rejections use the same error envelope as the handlers"
    );

    Ok(())
}

async fn submit(
    app: &Router,
    assignment_id: Uuid,
    body: Value,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post(format!("/assignments/{assignment_id}/submissions"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body)?))?;

    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await?;

    Ok((status, serde_json::from_slice(&bytes)?))
}
