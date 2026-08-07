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
async fn review_endpoints_cover_the_queue_and_the_review_decision() -> anyhow::Result<()> {
    let (_postgres, pool) = postgres_pool().await?;
    hw_bot::db::run_migrations(&pool).await?;

    let fixture = Fixture::insert(&pool).await?;
    let app = api::router(AppState::new("hw-bot-test", pool.clone()));

    let assignment_id = fixture.assignment.id().as_uuid();
    let teacher_id = fixture.teacher.telegram_user_id().as_i64();
    let first_student_id = fixture.student.telegram_user_id().as_i64();

    let (status, body) = take_next(&app, teacher_id).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, Value::Null, "an empty queue has no body to return");

    let (status, first_submission) =
        submit(&app, assignment_id, first_student_id, "pull/1").await?;
    assert_eq!(status, StatusCode::CREATED);

    let second_student = fixture
        .insert_student(&pool, 1002, "Second student")
        .await?;
    let second_student_id = second_student.telegram_user_id().as_i64();
    let (status, second_submission) =
        submit(&app, assignment_id, second_student_id, "pull/2").await?;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = take_next(&app, teacher_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["submission"]["id"], first_submission["id"],
        "the queue is served in submission order"
    );
    assert_eq!(body["submission"]["status"], json!("pending_review"));
    assert_eq!(body["assignment"]["id"], json!(assignment_id));
    assert_eq!(body["assignment"]["title"], json!("Rust ownership"));
    assert_eq!(body["assignment"]["max_attempts"], json!(2));
    assert_eq!(body["student"]["display_name"], json!("Student"));
    assert_eq!(body["student"]["telegram_user_id"], json!(first_student_id));

    let (status, body) = take_next(&app, first_student_id).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");

    let outsider = fixture
        .insert_teacher(&pool, 1003, "Outsider teacher")
        .await?;
    let outsider_id = outsider.telegram_user_id().as_i64();
    let (status, _) = take_next(&app, outsider_id).await?;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a teacher only sees submissions of the courses they teach"
    );

    let admin = fixture.insert_admin(&pool, 1004).await?;
    let (status, body) = take_next(&app, admin.telegram_user_id().as_i64()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["submission"]["id"], first_submission["id"],
        "an admin reviews every course"
    );

    let submission_id = first_submission["id"].as_str().unwrap().parse::<Uuid>()?;
    let (status, body) = review(&app, submission_id, outsider_id, "reject", "not mine").await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"]["code"], "permission_denied");

    let (status, body) = review(&app, submission_id, teacher_id, "reject", "   ").await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        body["error"]["code"], "empty_field",
        "a review comment is mandatory for both decisions"
    );

    let (status, body) = review(&app, submission_id, teacher_id, "maybe", "undecided").await?;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "invalid_request_body");

    let (status, body) = review(&app, Uuid::now_v7(), teacher_id, "accept", "good").await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "submission not found");

    let (status, body) = review(
        &app,
        submission_id,
        teacher_id,
        "reject",
        "  Add tests, then resubmit.  ",
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("rejected"));
    assert_eq!(body["reviewed_by"], json!(fixture.teacher.id().as_uuid()));
    assert_eq!(body["review_comment"], json!("Add tests, then resubmit."));
    assert!(body["reviewed_at"].as_str().is_some());

    let (status, body) =
        review(&app, submission_id, teacher_id, "accept", "changed my mind").await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"]["code"], "submission_is_not_pending_review",
        "a finished review is final, so a second reviewer loses the race"
    );

    let (status, body) = take_next(&app, teacher_id).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["submission"]["id"], second_submission["id"],
        "the reviewed submission leaves the queue"
    );

    let (status, retry) = submit(&app, assignment_id, first_student_id, "pull/3").await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        retry["attempt_number"],
        json!(2),
        "a rejected submission spends an attempt"
    );

    let retry_id = retry["id"].as_str().unwrap().parse::<Uuid>()?;
    let (status, body) = review(&app, retry_id, teacher_id, "accept", "95/100").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], json!("accepted"));
    assert_eq!(body["review_comment"], json!("95/100"));

    let (status, body) = submit(&app, assignment_id, first_student_id, "pull/4").await?;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body["error"]["code"], "submission_already_accepted",
        "an accepted submission cannot be redone even with attempts left"
    );

    Ok(())
}

async fn submit(
    app: &Router,
    assignment_id: Uuid,
    telegram_user_id: i64,
    path: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post(format!("/assignments/{assignment_id}/submissions"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "telegram_user_id": telegram_user_id,
            "content": format!("https://github.com/course/hw/{path}"),
        }))?))?;

    send(app, request).await
}

async fn take_next(app: &Router, telegram_user_id: i64) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post("/submissions/next")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "telegram_user_id": telegram_user_id,
        }))?))?;

    send(app, request).await
}

async fn review(
    app: &Router,
    submission_id: Uuid,
    telegram_user_id: i64,
    decision: &str,
    comment: &str,
) -> anyhow::Result<(StatusCode, Value)> {
    let request = Request::post(format!("/submissions/{submission_id}/review"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&json!({
            "telegram_user_id": telegram_user_id,
            "decision": decision,
            "comment": comment,
        }))?))?;

    send(app, request).await
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
