use std::time::Duration;

use chrono::Utc;
use hw_bot::health::{CriticalComponent, HealthState};
use hw_bot::telegram::{
    DeliveryOutcome, NotificationWorker, NotificationWorkerError, NotificationWorkerSettings,
    new_bot,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::TestApp;
use crate::harness::telegram::{telegram_me_body, telegram_message_response};

#[tokio::test]
async fn successful_telegram_delivery_completes_notification() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "95/100")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "✅ Работа принята.\n📘 Предмет: Backend development\n📚 Задание: Rust ownership\n\n💬 Комментарий преподавателя:\n95/100"
        })))
        .respond_with(telegram_message_response())
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
async fn delivery_keeps_its_lease_across_slow_claim_network_and_confirmation() {
    use sqlx::ConnectOptions;
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/slow").await;
    app.review(submission, app.teacher(), "accept", "Done")
        .await
        .ok();
    sqlx::query(
        "create function slow_outbox_write() returns trigger language plpgsql as $$
         begin perform pg_sleep(0.15); return new; end $$",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    sqlx::query(
        "create trigger slow_outbox_write before update on review_notification_outbox
         for each row execute function slow_outbox_write()",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let runtime = hw_bot::db::connect_runtime(
        app.pool.connect_options().to_url_lossy().as_str(),
        1,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(telegram_message_response().set_delay(Duration::from_millis(250)))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = NotificationWorker::new(
        runtime.clone(),
        new_bot("test-token", &telegram.uri(), Duration::from_secs(1)).unwrap(),
        NotificationWorkerSettings {
            request_timeout: Duration::from_secs(1),
            poll_interval: Duration::from_millis(10),
            lease_duration: Duration::from_secs(5),
            max_attempts: 3,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(2),
        },
    );
    let held = runtime.acquire().await.unwrap();
    let delivery = tokio::spawn(async move { worker.deliver_one(Utc::now()).await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    drop(held);
    wait_for_worker_request(&telegram, "/bottest-token/SendMessage").await;
    // Во время Telegram-запроса соединение свободно; подтверждение затем ждёт его заново.
    let held = runtime.acquire().await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!delivery.is_finished());
    let now = Utc::now();
    assert!(
        hw_bot::db::notifications::claim_next(
            &app.pool,
            now,
            uuid::Uuid::new_v4(),
            now + chrono::Duration::seconds(5)
        )
        .await
        .unwrap()
        .is_none()
    );
    drop(held);
    assert_eq!(delivery.await.unwrap().unwrap(), DeliveryOutcome::Delivered);
    runtime.close().await;
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
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "ok": false,
            "error_code": 500,
            "description": "internal error"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker(&app, &telegram, 3);
    let now = Utc::now();

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), worker.deliver_one(now))
            .await
            .expect("the request deadline also bounds teloxide's sleep after HTTP 5xx")
            .unwrap(),
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
    assert_eq!(state.last_error.as_deref(), Some("telegram_io_or_deadline"));
    assert_eq!(
        worker.deliver_one(now).await.unwrap(),
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
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "ok": false,
            "error_code": 403,
            "description": "Forbidden: bot was blocked by the user"
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

#[tokio::test]
async fn flood_control_pauses_the_worker_before_the_next_ready_notification() {
    use std::sync::{Arc, Mutex};
    use tokio::time::Instant;
    let app = TestApp::start().await;
    for content in ["pull/1", "pull/2"] {
        let id = app.queued_submission(app.student(), content).await;
        app.review(id, app.teacher(), "reject", "Fix it").await.ok();
    }
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(telegram_me_body()))
        .mount(&telegram)
        .await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(move |_: &wiremock::Request| {
            let mut calls = recorded.lock().unwrap();
            calls.push(Instant::now());
            if calls.len() == 1 {
                ResponseTemplate::new(429).set_body_json(json!({"ok": false, "error_code": 429, "description": "Too Many Requests: retry after 1",
                    "parameters": {"retry_after": 1}}))
            } else {
                telegram_message_response()
            }
        })
        .mount(&telegram)
        .await;
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(worker(&app, &telegram, 3).run(
        cancellation.clone(),
        HealthState::new(false, true).component(CriticalComponent::NotificationWorker),
    ));
    assert!(
        crate::harness::telegram::wait_until(Duration::from_secs(4), || async {
            calls.lock().unwrap().len() >= 2
        })
        .await
    );
    cancellation.cancel();
    task.await.unwrap().unwrap();
    let calls = calls.lock().unwrap();
    assert!(calls[1].duration_since(calls[0]) >= Duration::from_secs(1));
}

#[tokio::test]
async fn cancellation_interrupts_the_flood_control_cooldown() {
    let app = TestApp::start().await;
    let id = app.queued_submission(app.student(), "pull/1").await;
    app.review(id, app.teacher(), "reject", "Fix it").await.ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(telegram_me_body()))
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(
            ResponseTemplate::new(429).set_body_json(json!({"ok": false, "error_code": 429, "description": "Too Many Requests: retry after 60",
            "parameters": {"retry_after": 60}})),
        )
        .expect(1)
        .mount(&telegram)
        .await;
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(worker(&app, &telegram, 3).run(
        cancellation.clone(),
        HealthState::new(false, true).component(CriticalComponent::NotificationWorker),
    ));
    assert!(crate::harness::telegram::wait_until(Duration::from_secs(3), || async {
        sqlx::query_scalar::<_, bool>("select lease_token is null and last_error is not null from review_notification_outbox where submission_id = $1")
            .bind(id).fetch_one(&app.pool).await.unwrap()
    }).await);
    cancellation.cancel();
    tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn exhausting_delivery_attempts_marks_the_notification_failed() {
    let app = TestApp::start().await;
    let id = app.queued_submission(app.student(), "pull/1").await;
    app.review(id, app.teacher(), "reject", "Fix it").await.ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&telegram)
        .await;
    assert_eq!(
        worker(&app, &telegram, 1)
            .deliver_one(Utc::now())
            .await
            .unwrap(),
        DeliveryOutcome::Failed
    );
    let failed: bool = sqlx::query_scalar("select failed_at is not null and lease_token is null from review_notification_outbox where submission_id = $1")
        .bind(id).fetch_one(&app.pool).await.unwrap();
    assert!(failed);
}

#[tokio::test]
async fn invalid_credentials_preserve_the_notification_for_a_fixed_deployment() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "good")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false,
            "error_code": 401,
            "description": "Unauthorized"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker(&app, &telegram, 1);

    assert!(matches!(
        worker.deliver_one(Utc::now()).await,
        Err(NotificationWorkerError::InvalidCredentials)
    ));

    let state = sqlx::query!(
        r#"
        select attempt_count, lease_token, lease_expires_at, delivered_at, failed_at
        from review_notification_outbox
        where submission_id = $1
        "#,
        submission,
    )
    .fetch_one(&app.pool)
    .await
    .expect("load preserved notification");
    assert_eq!(state.attempt_count, 0);
    assert!(state.lease_token.is_none());
    assert!(state.lease_expires_at.is_none());
    assert!(state.delivered_at.is_none());
    assert!(state.failed_at.is_none());
}

#[tokio::test]
async fn run_becomes_ready_after_credentials_and_drops_readiness_on_exit() {
    let app = TestApp::start().await;
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(telegram_me_body()),
        )
        .expect(1)
        .mount(&telegram)
        .await;
    let worker = worker_with_poll_interval(&app, &telegram, Duration::from_secs(60));
    let health = HealthState::new(false, true);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(worker.run(
        cancellation.clone(),
        health.component(CriticalComponent::NotificationWorker),
    ));

    wait_for_worker_request(&telegram, "/bottest-token/GetMe").await;
    assert!(
        !health.is_ready_without_database(),
        "credentials must be validated before readiness"
    );
    wait_for_worker_readiness(&health).await;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_millis(300), task)
        .await
        .expect("idle worker reacts to cancellation without waiting for poll interval")
        .expect("worker task does not panic")
        .expect("worker exits cleanly");
    assert!(
        !health.is_ready_without_database(),
        "ready guard must be dropped when the worker exits"
    );
}

#[tokio::test]
async fn flood_control_without_retry_after_keeps_the_cooldown_after_attempts_are_exhausted() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Done")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false, "error_code": 429, "description": "Too Many Requests"
        })))
        .expect(1)
        .mount(&telegram)
        .await;

    assert_eq!(
        worker(&app, &telegram, 1)
            .deliver_one(Utc::now())
            .await
            .unwrap(),
        DeliveryOutcome::RateLimited {
            retry_after: Duration::from_secs(1)
        }
    );
    let failed: bool = sqlx::query_scalar(
        "select failed_at is not null and lease_token is null from review_notification_outbox where submission_id = $1"
    ).bind(submission).fetch_one(&app.pool).await.unwrap();
    assert!(failed);
}

#[tokio::test]
async fn malformed_delivery_response_is_retried_without_storing_its_raw_body() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Done")
        .await
        .ok();
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_string("private-comment-and-token"))
        .expect(1)
        .mount(&telegram)
        .await;

    assert_eq!(
        worker(&app, &telegram, 3)
            .deliver_one(Utc::now())
            .await
            .unwrap(),
        DeliveryOutcome::Rescheduled
    );
    let error: String = sqlx::query_scalar(
        "select last_error from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(error, "telegram_invalid_response");
}

#[tokio::test]
async fn malformed_startup_response_does_not_leak_into_the_process_error_or_mark_the_worker_ready()
{
    let app = TestApp::start().await;
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(200).set_body_string("private-comment-and-token"))
        .expect(1)
        .mount(&telegram)
        .await;
    let health = HealthState::new(false, true);
    let error = worker(&app, &telegram, 3)
        .run(
            CancellationToken::new(),
            health.component(CriticalComponent::NotificationWorker),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        NotificationWorkerError::Startup("telegram_invalid_response")
    ));
    assert!(!format!("{error:?}: {error}").contains("private-comment-and-token"));
    assert!(!health.is_ready_without_database());
}

async fn wait_for_worker_request(telegram: &MockServer, expected: &str) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let seen = telegram
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| request.url.path() == expected);
            if seen {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker sent the expected Telegram request");
}

async fn wait_for_worker_readiness(health: &HealthState) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !health.is_ready_without_database() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker became ready");
}

fn worker(app: &TestApp, telegram: &MockServer, max_attempts: u32) -> NotificationWorker {
    worker_with_settings(app, telegram, max_attempts, Duration::from_millis(10))
}

fn worker_with_poll_interval(
    app: &TestApp,
    telegram: &MockServer,
    poll_interval: Duration,
) -> NotificationWorker {
    worker_with_settings(app, telegram, 3, poll_interval)
}

fn worker_with_settings(
    app: &TestApp,
    telegram: &MockServer,
    max_attempts: u32,
    poll_interval: Duration,
) -> NotificationWorker {
    let telegram = new_bot("test-token", &telegram.uri(), Duration::from_secs(1))
        .expect("create Telegram client");

    NotificationWorker::new(
        app.pool.clone(),
        telegram,
        NotificationWorkerSettings {
            request_timeout: Duration::from_secs(1),
            poll_interval,
            lease_duration: Duration::from_secs(2),
            max_attempts,
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(10),
        },
    )
}
