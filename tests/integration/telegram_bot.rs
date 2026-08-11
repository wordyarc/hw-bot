use std::time::Duration;

use chrono::{DateTime, Utc};
use hw_bot::app::ApplicationContainer;
use hw_bot::bot::TelegramBot;
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::TestApp;

const BOT_MESSAGE_ID: i32 = 900;

#[tokio::test]
async fn teacher_creates_a_draft_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(7);
    let deadline_input = deadline.format("%d.%m.%Y %H:%M").to_string();
    let expected_deadline =
        chrono::NaiveDateTime::parse_from_str(&deadline_input, "%d.%m.%Y %H:%M")
            .unwrap()
            .and_utc();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), "menu:courses", BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("course:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("create:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        message_update(5, app.teacher(), 2, "Telegram draft"),
        message_update(
            6,
            app.teacher(),
            3,
            "Solve the task and send a pull request.",
        ),
        message_update(7, app.teacher(), 4, "3"),
        message_update(8, app.teacher(), 5, &deadline_input),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Telegram draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let callbacks = sent_callbacks(&telegram).await;
    telegram.verify().await;

    let (id, status, body, max_attempts, stored_deadline) =
        created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    assert_eq!(status, "draft");
    assert_eq!(body, "Solve the task and send a pull request.");
    assert_eq!(max_attempts, 3);
    assert_eq!(stored_deadline, expected_deadline);
    assert!(callbacks.contains(&format!("course:{}", app.course())));
    assert!(callbacks.contains(&format!("create:{}", app.course())));
    assert!(callbacks.contains(&format!("publish:{id}")));
    assert!(callbacks.iter().all(|callback| callback.len() <= 64));
}

#[tokio::test]
async fn teacher_changes_an_assignment_and_grants_attempts_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(14);
    let deadline_input = deadline.format("%d.%m.%Y %H:%M").to_string();
    let expected_deadline =
        chrono::NaiveDateTime::parse_from_str(&deadline_input, "%d.%m.%Y %H:%M")
            .unwrap()
            .and_utc();
    let assignment = app.assignment();
    let student = app.fixture.student.id().as_uuid();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), "menu:courses", BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("course:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("manage:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("archive:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            6,
            app.teacher(),
            &format!("publish:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            7,
            app.teacher(),
            &format!("deadline:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        message_update(8, app.teacher(), 2, &deadline_input),
        callback_update(
            9,
            app.teacher(),
            &format!("attempts:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            10,
            app.teacher(),
            &format!("student:{student}"),
            BOT_MESSAGE_ID,
        ),
        message_update(11, app.teacher(), 3, "2"),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let granted = wait_for_extra_attempts(&app.pool, assignment, student, 2).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let callbacks = sent_callbacks(&telegram).await;
    telegram.verify().await;

    assert!(
        granted,
        "extra attempts were not granted; requests:\n{requests}"
    );
    let row = sqlx::query_as::<_, (String, DateTime<Utc>, i64)>(
        "select status, deadline_at, revision from assignments where id = $1",
    )
    .bind(assignment)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "published");
    assert_eq!(row.1, expected_deadline);
    assert_eq!(
        row.2, 3,
        "archive, re-publication and deadline each write once"
    );
    for expected in [
        format!("course:{}", app.course()),
        format!("manage:{assignment}"),
        format!("archive:{assignment}"),
        format!("publish:{assignment}"),
        format!("deadline:{assignment}"),
        format!("attempts:{assignment}"),
        format!("student:{student}"),
    ] {
        assert!(callbacks.contains(&expected), "missing callback {expected}");
    }
    assert!(callbacks.iter().all(|callback| callback.len() <= 64));
}

#[tokio::test]
async fn stale_assignment_button_does_not_mutate_the_assignment() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), "menu:courses", BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("course:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("manage:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("archive:{assignment}"),
            BOT_MESSAGE_ID - 1,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Эта кнопка устарела. Откройте /menu ещё раз."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("stale callback was not rejected; requests:\n{requests}"));

    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(assignment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "published");
}

fn start_bot(app: &TestApp, telegram: &MockServer) -> (CancellationToken, JoinHandle<()>) {
    let bot = TelegramBot::new(
        ApplicationContainer::new(app.pool.clone()),
        "test-token",
        &telegram.uri(),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(bot.run(cancellation.clone()));

    (cancellation, task)
}

async fn stop_bot(cancellation: CancellationToken, task: &mut JoinHandle<()>) {
    cancellation.cancel();
    match tokio::time::timeout(Duration::from_secs(3), &mut *task).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => panic!("Telegram bot task failed: {error}"),
        Err(_) => {
            task.abort();
            panic!("Telegram bot did not stop");
        }
    }
}

async fn telegram_server(updates: Vec<Value>) -> MockServer {
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetWebhookInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {
                "url": "",
                "has_custom_certificate": false,
                "pending_update_count": 0
            }
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {
                "id": 999,
                "is_bot": true,
                "first_name": "Test Bot",
                "username": "test_bot",
                "can_join_groups": true,
                "can_read_all_group_messages": false,
                "supports_inline_queries": false,
                "has_main_web_app": false
            }
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetUpdates"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let first_poll = body["offset"].as_i64().unwrap_or(0) == 0;
            let result = if first_poll {
                updates.clone()
            } else {
                Vec::new()
            };
            let response =
                ResponseTemplate::new(200).set_body_json(json!({ "ok": true, "result": result }));

            if !first_poll && body["timeout"].as_u64() == Some(10) {
                response.set_delay(Duration::from_secs(5))
            } else {
                response
            }
        })
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/AnswerCallbackQuery"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": true
        })))
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(telegram_message_response())
        .with_priority(10)
        .mount(&telegram)
        .await;

    telegram
}

fn telegram_message_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "ok": true,
        "result": {
            "message_id": BOT_MESSAGE_ID,
            "date": 0,
            "chat": { "id": 1000, "type": "private" },
            "from": { "id": 999, "is_bot": true, "first_name": "Test Bot" },
            "text": "response"
        }
    }))
}

fn message_update(update_id: u32, user_id: i64, message_id: i32, text: &str) -> Value {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": message_id,
            "date": 0,
            "chat": { "id": user_id, "type": "private" },
            "from": {
                "id": user_id,
                "is_bot": false,
                "first_name": "Teacher"
            },
            "text": text
        }
    })
}

fn callback_update(update_id: u32, user_id: i64, data: &str, message_id: i32) -> Value {
    json!({
        "update_id": update_id,
        "callback_query": {
            "id": format!("callback-{update_id}"),
            "from": {
                "id": user_id,
                "is_bot": false,
                "first_name": "Teacher"
            },
            "message": {
                "message_id": message_id,
                "date": 1,
                "chat": { "id": user_id, "type": "private" },
                "from": { "id": 999, "is_bot": true, "first_name": "Test Bot" },
                "text": "Choose an action"
            },
            "chat_instance": "test-chat",
            "data": data
        }
    })
}

async fn wait_for_created_assignment(
    pool: &PgPool,
    title: &str,
) -> Option<(Uuid, String, String, i16, DateTime<Utc>)> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = sqlx::query_as::<_, (Uuid, String, String, i16, DateTime<Utc>)>(
                "select id, status, body, max_attempts, deadline_at from assignments where title = $1",
            )
            .bind(title)
            .fetch_optional(pool)
            .await
            .unwrap();
            if row.is_some() {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .ok()
    .flatten()
}

async fn wait_for_extra_attempts(
    pool: &PgPool,
    assignment_id: Uuid,
    student_id: Uuid,
    expected: i16,
) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let amount = sqlx::query_scalar::<_, i16>(
                "select extra_attempts from student_assignment_progress where assignment_id = $1 and student_id = $2",
            )
            .bind(assignment_id)
            .bind(student_id)
            .fetch_optional(pool)
            .await
            .unwrap();
            if amount == Some(expected) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

async fn recorded_requests(telegram: &MockServer) -> String {
    telegram
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .map(|request| {
            format!(
                "{} {} {}",
                request.method,
                request.url.path(),
                String::from_utf8_lossy(&request.body)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn sent_callbacks(telegram: &MockServer) -> Vec<String> {
    let mut callbacks = Vec::new();
    for request in telegram.received_requests().await.unwrap() {
        if request.url.path() != "/bottest-token/SendMessage" {
            continue;
        }
        let body: Value = request.body_json().unwrap();
        let Some(rows) = body["reply_markup"]["inline_keyboard"].as_array() else {
            continue;
        };
        for button in rows.iter().flat_map(|row| row.as_array().unwrap()) {
            if let Some(callback) = button["callback_data"].as_str() {
                callbacks.push(callback.to_owned());
            }
        }
    }

    callbacks
}
