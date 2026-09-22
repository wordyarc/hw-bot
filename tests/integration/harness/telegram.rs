use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use hw_bot::app::ApplicationContainer;
use hw_bot::bot::{BotTimezone, TelegramBot, TelegramBotSettings, callbacks};
use hw_bot::health::{CriticalComponent, HealthState};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::TestApp;

/// Все ожидания опрашивают состояние раз в 10 мс. При шаге 20 мс под нагрузкой иногда
/// пропускалось самое короткое окно: 300 мс на подтверждение polling offset.
const POLL_STEP: Duration = Duration::from_millis(10);

pub(crate) async fn wait_until<F, Fut>(timeout: Duration, mut probe: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    tokio::time::timeout(timeout, async {
        loop {
            if probe().await {
                return;
            }
            tokio::time::sleep(POLL_STEP).await;
        }
    })
    .await
    .is_ok()
}

pub(crate) async fn wait_for_some<T, F, Fut>(timeout: Duration, mut probe: F) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    tokio::time::timeout(timeout, async {
        loop {
            if let Some(value) = probe().await {
                return value;
            }
            tokio::time::sleep(POLL_STEP).await;
        }
    })
    .await
    .ok()
}

pub(crate) const BOT_MESSAGE_ID: i32 = 900;

pub(crate) fn cb(prefix: &str, value: impl std::fmt::Display) -> String {
    format!("{prefix}{value}")
}

pub(crate) const HELD_PERMIT: Duration = Duration::from_millis(600);

pub(crate) const DOCUMENT_FIXTURE: &[u8] = include_bytes!("../fixtures/assignment-note.txt");

pub(crate) const IMAGE_FIXTURE: &[u8] = include_bytes!("../fixtures/pixel.ppm");

pub(crate) fn start_bot(
    app: &TestApp,
    telegram: &MockServer,
) -> (
    CancellationToken,
    JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    start_bot_with_settings(app, telegram, bot_settings_with(|_| {}))
}

pub(crate) fn start_bot_with_settings(
    app: &TestApp,
    telegram: &MockServer,
    settings: TelegramBotSettings,
) -> (
    CancellationToken,
    JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    start_bot_with_application(telegram, app.application.clone(), settings)
}

pub(crate) fn start_bot_with_application(
    telegram: &MockServer,
    application: ApplicationContainer,
    settings: TelegramBotSettings,
) -> (
    CancellationToken,
    JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    let bot = TelegramBot::new(
        application,
        "test-token",
        &telegram.uri(),
        settings,
        bot_timezone(),
        crate::harness::invite_codec(),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let health = HealthState::new(true, false);
    let task = tokio::spawn(bot.run(
        cancellation.clone(),
        health.component(CriticalComponent::TelegramBot),
    ));

    (cancellation, task)
}

pub(crate) fn file_upload_updates(app: &TestApp, assignment: Uuid, file: Value) -> Vec<Value> {
    vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::MENU_COURSES, BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("{}{}", callbacks::COURSE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("{}{assignment}", callbacks::MANAGE_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("{}{assignment}", callbacks::FILES_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), callbacks::FILE_ADD, BOT_MESSAGE_ID),
        file,
    ]
}

pub(crate) fn bot_timezone() -> BotTimezone {
    BotTimezone::new(Tz::Asia__Vladivostok)
}

pub(crate) fn deadline_input(deadline: DateTime<Utc>) -> String {
    deadline
        .with_timezone(&Tz::Asia__Vladivostok)
        .format("%d.%m.%Y %H:%M")
        .to_string()
}

pub(crate) fn expected_deadline(input: &str) -> DateTime<Utc> {
    bot_timezone()
        .parse_deadline(input)
        .unwrap_or_else(|| panic!("{input} is a valid deadline"))
}

pub(crate) fn bot_settings_with(
    update: impl FnOnce(&mut TelegramBotSettings),
) -> TelegramBotSettings {
    let mut settings = TelegramBotSettings {
        request_timeout: Duration::from_secs(2),
        polling_timeout: Duration::from_secs(10),
        rate_limit_replenish: Duration::from_secs(1),
        rate_limit_burst: 100,
        max_concurrent_updates: 32,
        file_transfer_timeout: Duration::from_secs(5),
        max_concurrent_file_transfers: 2,
    };
    update(&mut settings);
    settings
}

pub(crate) async fn stop_bot(
    cancellation: CancellationToken,
    task: &mut JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    cancellation.cancel();
    match tokio::time::timeout(Duration::from_secs(3), &mut *task).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => panic!("Telegram bot failed: {error}"),
        Ok(Err(error)) => panic!("Telegram bot task failed: {error}"),
        Err(_) => {
            task.abort();
            panic!("Telegram bot did not stop");
        }
    }
}

pub(crate) async fn telegram_server(updates: Vec<Value>) -> MockServer {
    let telegram = MockServer::start().await;
    mount_startup_mocks(&telegram).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(200).set_body_json(telegram_me_body()))
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
    Mock::given(method("POST"))
        .and(path("/bottest-token/EditMessageReplyMarkup"))
        .respond_with(telegram_message_response())
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/EditMessageText"))
        .respond_with(telegram_message_response())
        .mount(&telegram)
        .await;

    telegram
}

pub(crate) async fn mount_startup_mocks(telegram: &MockServer) {
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
        .mount(telegram)
        .await;
    for endpoint in ["SetMyCommands", "SetChatMenuButton"] {
        Mock::given(method("POST"))
            .and(path(format!("/bottest-token/{endpoint}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": true
            })))
            .mount(telegram)
            .await;
    }
}

pub(crate) fn telegram_me_body() -> Value {
    json!({
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
    })
}

pub(crate) fn telegram_message_response() -> ResponseTemplate {
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

pub(crate) fn telegram_document_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "ok": true,
        "result": {
            "message_id": BOT_MESSAGE_ID,
            "date": 0,
            "chat": { "id": 1000, "type": "private" },
            "from": { "id": 999, "is_bot": true, "first_name": "Test Bot" },
            "document": {
                "file_id": "sent-document",
                "file_unique_id": "sent-document-unique",
                "file_size": DOCUMENT_FIXTURE.len(),
                "file_name": "assignment-note.txt"
            }
        }
    }))
}

pub(crate) fn telegram_photo_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "ok": true,
        "result": {
            "message_id": BOT_MESSAGE_ID,
            "date": 0,
            "chat": { "id": 1000, "type": "private" },
            "from": { "id": 999, "is_bot": true, "first_name": "Test Bot" },
            "photo": [{
                "file_id": "sent-photo",
                "file_unique_id": "sent-photo-unique",
                "file_size": IMAGE_FIXTURE.len(),
                "width": 1,
                "height": 1
            }]
        }
    }))
}

pub(crate) async fn expect_sent_messages(
    telegram: &MockServer,
    count: u64,
    delay: Duration,
) -> wiremock::MockGuard {
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(telegram_message_response().set_delay(delay))
        .expect(count)
        .with_priority(1)
        .mount_as_scoped(telegram)
        .await
}

pub(crate) async fn wait_for_sent_message_count(telegram: &MockServer, expected: usize) {
    assert!(
        wait_until(Duration::from_secs(2), || async {
            sent_message_count(telegram).await >= expected
        })
        .await,
        "Telegram request is sent"
    );
}

pub(crate) async fn wait_for_poll_offset(telegram: &MockServer, offset: i64) -> bool {
    wait_until(Duration::from_millis(300), || async {
        poll_offsets(telegram).await.contains(&offset)
    })
    .await
}

pub(crate) async fn last_poll_request(telegram: &MockServer) -> Value {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/bottest-token/GetUpdates")
        .filter_map(|request| request.body_json::<Value>().ok())
        .next_back()
        .expect("the bot polled Telegram at least once")
}

pub(crate) async fn poll_offsets(telegram: &MockServer) -> Vec<i64> {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/bottest-token/GetUpdates")
        .filter_map(|request| request.body_json::<Value>().ok())
        .map(|body| body["offset"].as_i64().unwrap_or(0))
        .collect()
}

pub(crate) async fn sent_message_count(telegram: &MockServer) -> usize {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/bottest-token/SendMessage")
        .count()
}

pub(crate) fn message_update(update_id: u32, user_id: i64, message_id: i32, text: &str) -> Value {
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

pub(crate) fn document_update(
    update_id: u32,
    user_id: i64,
    message_id: i32,
    file_id: &str,
    filename: &str,
    size: usize,
) -> Value {
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
            "document": {
                "file_id": file_id,
                "file_unique_id": format!("{file_id}-unique"),
                "file_size": size,
                "file_name": filename,
                "mime_type": "text/plain"
            }
        }
    })
}

pub(crate) fn photo_update(update_id: u32, user_id: i64, message_id: i32) -> Value {
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
            "photo": [
                {
                    "file_id": "photo-small",
                    "file_unique_id": "photo-small-unique",
                    "file_size": 10,
                    "width": 90,
                    "height": 90
                },
                {
                    "file_id": "photo-large",
                    "file_unique_id": "photo-large-unique",
                    "file_size": IMAGE_FIXTURE.len(),
                    "width": 800,
                    "height": 600
                }
            ]
        }
    })
}

pub(crate) async fn mock_file_download(
    telegram: &MockServer,
    file_id: &str,
    file_path: &str,
    content: &'static [u8],
) {
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetFile"))
        .and(body_json(json!({ "file_id": file_id })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {
                "file_id": file_id,
                "file_unique_id": format!("{file_id}-unique"),
                "file_size": content.len(),
                "file_path": file_path
            }
        })))
        .expect(1)
        .mount(telegram)
        .await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/file/bottest-token/{}",
            file_path.replace('/', "%2F")
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(content))
        .expect(1)
        .mount(telegram)
        .await;
}

pub(crate) async fn wait_for_request_count(telegram: &MockServer, expected: &str, count: usize) {
    let reached = wait_until(Duration::from_secs(5), || async {
        request_count(telegram, expected).await >= count
    })
    .await;
    if !reached {
        panic!(
            "Telegram request {expected} was not sent {count} times; requests:\n{}",
            recorded_requests(telegram).await
        );
    }
}

async fn request_count(telegram: &MockServer, path: &str) -> usize {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == path)
        .count()
}

pub(crate) async fn wait_for_request_path(telegram: &MockServer, expected: &str) {
    let reached = wait_until(Duration::from_secs(5), || async {
        request_count(telegram, expected).await > 0
    })
    .await;
    if !reached {
        panic!(
            "Telegram request {expected} was not sent; requests:\n{}",
            recorded_requests(telegram).await
        );
    }
}

pub(crate) async fn wait_for_stored_file(
    pool: &PgPool,
    filename: &str,
) -> Option<(Vec<u8>, i64, String)> {
    wait_for_some(Duration::from_secs(5), || async {
        sqlx::query_as::<_, (Vec<u8>, i64, String)>(
            "select content, size_bytes, presentation from stored_files where filename = $1",
        )
        .bind(filename)
        .fetch_optional(pool)
        .await
        .unwrap()
    })
    .await
}

pub(crate) async fn wait_for_presentation(
    pool: &PgPool,
    presentation: &str,
) -> Option<(Vec<u8>, String)> {
    wait_for_some(Duration::from_secs(5), || async {
        sqlx::query_as::<_, (Vec<u8>, String)>(
            "select content, filename from stored_files where presentation = $1",
        )
        .bind(presentation)
        .fetch_optional(pool)
        .await
        .unwrap()
    })
    .await
}

pub(crate) async fn wait_for_file_count(pool: &PgPool, assignment_id: Uuid, expected: i64) {
    assert!(
        wait_until(Duration::from_secs(5), || async {
            stored_file_count(pool, assignment_id).await == expected
        })
        .await,
        "stored file count reaches expected value"
    );
}

pub(crate) async fn stored_file_count(pool: &PgPool, assignment_id: Uuid) -> i64 {
    sqlx::query_scalar("select count(*) from stored_files where assignment_id = $1")
        .bind(assignment_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub(crate) fn callback_update(update_id: u32, user_id: i64, data: &str, message_id: i32) -> Value {
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

#[derive(sqlx::FromRow)]
pub(crate) struct CreatedAssignment {
    pub(crate) id: Uuid,
    pub(crate) status: String,
    pub(crate) body: String,
    pub(crate) max_attempts: i16,
    pub(crate) max_score: Option<i16>,
    pub(crate) deadline_at: DateTime<Utc>,
}

pub(crate) async fn wait_for_created_assignment(
    pool: &PgPool,
    title: &str,
) -> Option<CreatedAssignment> {
    wait_for_some(Duration::from_secs(5), || async {
        sqlx::query_as::<_, CreatedAssignment>(
            "select id, status, body, max_attempts, max_score, deadline_at from assignments where title = $1",
        )
        .bind(title)
        .fetch_optional(pool)
        .await
        .unwrap()
    })
    .await
}

pub(crate) async fn wait_for_review_result(
    pool: &PgPool,
    submission: Uuid,
) -> Option<(String, Option<i16>, Option<i16>)> {
    wait_for_some(Duration::from_secs(5), || async {
        sqlx::query_as::<_, (String, Option<i16>, Option<i16>)>(
            "select status, raw_score, final_score from submissions where id = $1 and status <> 'pending_review'",
        )
        .bind(submission)
        .fetch_optional(pool)
        .await
        .unwrap()
    })
    .await
}

pub(crate) async fn wait_for_extra_attempts(
    pool: &PgPool,
    assignment_id: Uuid,
    student_id: Uuid,
    expected: i16,
) -> bool {
    wait_until(Duration::from_secs(5), || async {
        sqlx::query_scalar::<_, i16>(
            "select extra_attempts from student_assignment_progress where assignment_id = $1 and student_id = $2",
        )
        .bind(assignment_id)
        .bind(student_id)
        .fetch_optional(pool)
        .await
        .unwrap()
            == Some(expected)
    })
    .await
}

pub(crate) async fn recorded_requests(telegram: &MockServer) -> String {
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

pub(crate) async fn sent_callbacks(telegram: &MockServer) -> Vec<String> {
    let mut callbacks = Vec::new();
    for request in telegram.received_requests().await.unwrap() {
        if !matches!(
            request.url.path(),
            "/bottest-token/SendMessage"
                | "/bottest-token/EditMessageText"
                | "/bottest-token/EditMessageReplyMarkup"
        ) {
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

pub(crate) async fn expect_text(
    telegram: &MockServer,
    chat_id: i64,
    text: &str,
) -> wiremock::MockGuard {
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({ "chat_id": chat_id, "text": text })))
        .respond_with(telegram_message_response())
        .expect(1..)
        .with_priority(1)
        .mount_as_scoped(telegram)
        .await
}

pub(crate) async fn wait_for_group(pool: &PgPool, telegram_user_id: i64) -> Option<Uuid> {
    wait_for_some(Duration::from_secs(3), || async {
        sqlx::query_scalar::<_, Option<Uuid>>(
            "select student_group_id from users where telegram_user_id = $1",
        )
        .bind(telegram_user_id)
        .fetch_optional(pool)
        .await
        .unwrap()
    })
    .await
    .flatten()
}
