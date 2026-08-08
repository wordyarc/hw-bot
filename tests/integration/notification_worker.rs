use std::time::Duration;

use chrono::Utc;
use hw_bot::app::{DeliveryOutcome, NotificationWorker, NotificationWorkerSettings};
use hw_bot::telegram::TelegramClient;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::TestApp;

#[tokio::test]
async fn successful_telegram_delivery_completes_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "95/100")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "Работа принята.\nЗадание: Rust ownership\n\nКомментарий преподавателя:\n95/100"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "ok": true })))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker(&app, &telegram, 3);

    assert_eq!(
        worker.deliver_one(Utc::now()).await.unwrap(),
        DeliveryOutcome::Delivered
    );

    let delivered: bool = sqlx::query_scalar(
        "select delivered_at is not null from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .expect("load delivery status");
    assert!(delivered);
}

#[tokio::test]
async fn transient_telegram_error_reschedules_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "reject", "fix it")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendMessage"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "ok": false,
            "error_code": 500,
            "description": "internal error"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker(&app, &telegram, 3);

    assert_eq!(
        worker.deliver_one(Utc::now()).await.unwrap(),
        DeliveryOutcome::Rescheduled
    );

    let state = sqlx::query!(
        r#"
        select attempt_count, delivered_at, failed_at, last_error
        from review_notification_outbox
        where submission_id = $1
        "#,
        submission,
    )
    .fetch_one(&app.pool)
    .await
    .expect("load retry state");
    assert_eq!(state.attempt_count, 1);
    assert!(state.delivered_at.is_none());
    assert!(state.failed_at.is_none());
    assert!(
        state
            .last_error
            .is_some_and(|error| error.contains("internal error"))
    );
    assert_eq!(
        worker.deliver_one(Utc::now()).await.unwrap(),
        DeliveryOutcome::Idle,
        "a retry must wait until available_at"
    );
}

#[tokio::test]
async fn permanent_telegram_error_fails_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "good")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/sendMessage"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "ok": false,
            "error_code": 403,
            "description": "bot was blocked by the user"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker(&app, &telegram, 3);

    assert_eq!(
        worker.deliver_one(Utc::now()).await.unwrap(),
        DeliveryOutcome::Failed
    );

    let failed: bool = sqlx::query_scalar(
        "select failed_at is not null from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .expect("load failure status");
    assert!(failed);
}

fn worker(app: &TestApp, telegram: &MockServer, max_attempts: u32) -> NotificationWorker {
    let telegram = TelegramClient::new(telegram.uri(), "test-token", Duration::from_secs(1))
        .expect("create Telegram client");

    NotificationWorker::new(
        app.pool.clone(),
        telegram,
        NotificationWorkerSettings {
            poll_interval: Duration::from_millis(10),
            lease_duration: Duration::from_secs(2),
            max_attempts,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(10),
        },
    )
}
