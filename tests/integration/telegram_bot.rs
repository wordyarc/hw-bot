use std::time::Duration;

use chrono::{DateTime, FixedOffset, NaiveTime, Utc};
use hw_bot::app::ApplicationContainer;
use hw_bot::bot::{BotTimezone, TelegramBot, TelegramBotSettings};
use hw_bot::health::{CriticalComponent, HealthState};
use serde_json::{Value, json};
use sqlx::PgPool;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::{TestApp, test_file_policy};

const BOT_MESSAGE_ID: i32 = 900;
const ATTEMPTS_CALLBACK: &str = "maxattempts:3";
const DOCUMENT_FIXTURE: &[u8] = include_bytes!("fixtures/assignment-note.txt");
const IMAGE_FIXTURE: &[u8] = include_bytes!("fixtures/pixel.ppm");

#[tokio::test]
async fn teacher_creates_a_draft_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(7);
    let deadline_input = deadline_input(deadline);
    let expected_deadline = expected_deadline(&deadline_input);
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
        callback_update(9, app.teacher(), "file:finish", BOT_MESSAGE_ID),
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
async fn a_prompt_hides_the_menu_and_the_finished_step_brings_it_back() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("submit:{}", app.assignment()),
            BOT_MESSAGE_ID,
        ),
        message_update(3, app.student(), 2, "https://example.test/pull/1"),
    ];
    let telegram = telegram_server(updates).await;
    let prompt = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "Пришлите ссылку или текст сдачи одним сообщением:\n\n/cancel — отменить",
            "reply_markup": { "remove_keyboard": true }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let restored = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "✅ Работа отправлена на проверку. Место в очереди видно в «⏳ Очередь проверки».",
            "reply_markup": {
                "keyboard": [
                    [{ "text": "📚 Мои задания" }, { "text": "⏳ Очередь проверки" }],
                    [{ "text": "🗄 Архив" }, { "text": "👤 Профиль" }],
                    [{ "text": "❓ Помощь" }]
                ],
                "is_persistent": true,
                "resize_keyboard": true,
                "input_field_placeholder": "Выберите пункт меню"
            }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let asked = tokio::time::timeout(Duration::from_secs(5), prompt.wait_until_satisfied()).await;
    let returned =
        tokio::time::timeout(Duration::from_secs(5), restored.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    asked.unwrap_or_else(|_| panic!("the prompt did not hide the menu; requests:\n{requests}"));
    returned.unwrap_or_else(|_| panic!("the menu did not come back; requests:\n{requests}"));
}

#[tokio::test]
async fn the_bottom_menu_opens_a_screen_even_in_the_middle_of_a_dialogue() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("submit:{}", app.assignment()),
            BOT_MESSAGE_ID,
        ),
        message_update(3, app.student(), 2, "⏳ Очередь проверки"),
    ];
    let telegram = telegram_server(updates).await;
    let queue = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "⏳ Сейчас ни одна ваша работа не ждёт проверки.",
            "reply_markup": {
                "inline_keyboard": [
                    [{ "text": "📚 Мои задания", "callback_data": "menu:assignments" }],
                    [
                        { "text": "⏳ Очередь проверки", "callback_data": "menu:queue" },
                        { "text": "🗄 Архив", "callback_data": "menu:archive" }
                    ],
                    [{ "text": "👤 Профиль", "callback_data": "menu:profile" }]
                ]
            }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), queue.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the queue screen did not open; requests:\n{requests}"));

    let submissions: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        submissions, 0,
        "a menu button is not the text of a submission"
    );
}

#[tokio::test]
async fn replayable_inline_navigation_cancels_the_previous_input_flow() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("submit:{}", app.assignment()),
            BOT_MESSAGE_ID,
        ),
        callback_update(3, app.student(), "menu:assignments", BOT_MESSAGE_ID - 5),
        message_update(4, app.student(), 2, "https://example.test/pull/1"),
    ];
    let telegram = telegram_server(updates).await;
    let idle = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "Используйте меню под полем ввода или команду /menu."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), idle.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the old flow remained active; requests:\n{requests}"));

    let submissions: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(submissions, 0, "navigation must cancel submission input");
}

#[tokio::test]
async fn teacher_picks_a_deadline_with_buttons_instead_of_typing_a_date() {
    let app = TestApp::start().await;
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
        message_update(5, app.teacher(), 2, "Deadline in a week"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        callback_update(7, app.teacher(), ATTEMPTS_CALLBACK, BOT_MESSAGE_ID),
        callback_update(8, app.teacher(), "dl:days:7", BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Deadline in a week").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    assert!(
        requests.contains(&format!(r#""callback_data":"{ATTEMPTS_CALLBACK}""#)),
        "the keyboard must send exactly what the handler matches; requests:\n{requests}"
    );
    let (_, status, _, max_attempts, deadline) =
        created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    assert_eq!(status, "draft");
    assert_eq!(
        max_attempts, 3,
        "attempts came from a button, not from text"
    );
    let local = deadline.with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap());
    assert_eq!(
        local.date_naive(),
        (Utc::now() + chrono::Duration::days(7))
            .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
            .date_naive(),
        "«через неделю» is a week from today in the configured timezone"
    );
    assert_eq!(
        local.format("%H:%M").to_string(),
        "23:59",
        "presets expire at the end of the local day"
    );
}

#[tokio::test]
async fn teacher_picks_a_deadline_from_the_calendar() {
    let app = TestApp::start().await;
    let picked = (Utc::now() + chrono::Duration::days(3))
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .date_naive();
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
        message_update(5, app.teacher(), 2, "Calendar draft"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        callback_update(7, app.teacher(), "maxattempts:1", BOT_MESSAGE_ID),
        callback_update(8, app.teacher(), "dl:calendar", BOT_MESSAGE_ID),
        callback_update(
            9,
            app.teacher(),
            &format!("cal:pick:{}", picked.format("%Y-%m-%d")),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Calendar draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let (_, _, _, _, deadline) =
        created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    assert_eq!(
        deadline
            .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
            .date_naive(),
        picked
    );
    assert!(
        requests.contains("EditMessageReplyMarkup"),
        "the calendar replaces the keyboard in place; requests:\n{requests}"
    );
}

#[tokio::test]
async fn teacher_adds_and_opens_a_real_document_during_draft_creation() {
    let app = TestApp::start().await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(7));
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
        message_update(5, app.teacher(), 2, "Draft with attachment"),
        message_update(6, app.teacher(), 3, "Read the attached notes."),
        message_update(7, app.teacher(), 4, "2"),
        message_update(8, app.teacher(), 5, &deadline),
        callback_update(9, app.teacher(), "file:add", BOT_MESSAGE_ID),
        document_update(
            10,
            app.teacher(),
            6,
            "fixture-document",
            "assignment-note.txt",
            DOCUMENT_FIXTURE.len(),
        ),
        callback_update(11, app.teacher(), "file:open:0", BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    mock_file_download(
        &telegram,
        "fixture-document",
        "files/assignment-note.txt",
        DOCUMENT_FIXTURE,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response())
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let stored = wait_for_stored_file(&app.pool, "assignment-note.txt").await;
    wait_for_request_path(&telegram, "/bottest-token/SendDocument").await;
    stop_bot(cancellation, &mut task).await;
    let requests = telegram.received_requests().await.unwrap();

    let (content, size_bytes, presentation) = stored.expect("document was stored");
    assert_eq!(content, DOCUMENT_FIXTURE);
    assert_eq!(size_bytes, DOCUMENT_FIXTURE.len() as i64);
    assert_eq!(presentation, "document");
    let upload = requests
        .iter()
        .find(|request| request.url.path() == "/bottest-token/SendDocument")
        .expect("SendDocument request");
    assert!(
        upload
            .body
            .windows(DOCUMENT_FIXTURE.len())
            .any(|window| window == DOCUMENT_FIXTURE)
    );
}

#[tokio::test]
async fn teacher_adds_and_opens_the_largest_photo_variant() {
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
            &format!("files:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), "file:add", BOT_MESSAGE_ID),
        photo_update(7, app.teacher(), 2),
        callback_update(8, app.teacher(), "file:open:0", BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    mock_file_download(&telegram, "photo-large", "photos/pixel.jpg", IMAGE_FIXTURE).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendPhoto"))
        .respond_with(telegram_photo_response())
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let stored = wait_for_presentation(&app.pool, "image").await;
    wait_for_request_path(&telegram, "/bottest-token/SendPhoto").await;
    stop_bot(cancellation, &mut task).await;

    let (content, filename) = stored.expect("photo was stored");
    assert_eq!(content, IMAGE_FIXTURE);
    assert!(filename.starts_with("photo-"));
    assert!(filename.ends_with(".jpg"));
    let requests = recorded_requests(&telegram).await;
    assert!(requests.contains("\"file_id\":\"photo-large\""));
    assert!(!requests.contains("\"file_id\":\"photo-small\""));
}

#[tokio::test]
async fn declared_oversized_document_is_rejected_without_get_file() {
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
            &format!("files:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), "file:add", BOT_MESSAGE_ID),
        document_update(
            7,
            app.teacher(),
            2,
            "too-large",
            "too-large.bin",
            5 * 1024 * 1024 + 1,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let rejected = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Файл превышает допустимый размер."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), rejected.wait_until_satisfied())
        .await
        .expect("oversized file is rejected");
    stop_bot(cancellation, &mut task).await;
    let requests = telegram.received_requests().await.unwrap();

    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/bottest-token/GetFile")
    );
}

#[tokio::test]
async fn oversized_download_stream_is_rejected_without_a_database_row() {
    use hw_bot::app::FilePolicy;

    let app = TestApp::start().await;
    let assignment = app.assignment();
    let max_size = DOCUMENT_FIXTURE.len() - 1;
    let updates = file_upload_updates(
        &app,
        assignment,
        document_update(
            7,
            app.teacher(),
            2,
            "stream-too-large",
            "too-large.txt",
            max_size,
        ),
    );
    let telegram = telegram_server(updates).await;
    mock_file_download(
        &telegram,
        "stream-too-large",
        "files/too-large.txt",
        DOCUMENT_FIXTURE,
    )
    .await;
    let failed = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Не удалось скачать файл. Попробуйте ещё раз."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let application = ApplicationContainer::new(
        app.pool.clone(),
        FilePolicy {
            max_file_size_bytes: max_size,
            ..test_file_policy()
        },
    );
    let (cancellation, mut task) =
        start_bot_with_application(&telegram, application, bot_settings_with(|_| {}));

    tokio::time::timeout(Duration::from_secs(5), failed.wait_until_satisfied())
        .await
        .expect("oversized stream is rejected");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(stored_file_count(&app.pool, assignment).await, 0);
}

#[tokio::test]
async fn failed_download_does_not_create_a_database_row() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = file_upload_updates(
        &app,
        assignment,
        document_update(
            7,
            app.teacher(),
            2,
            "download-fails",
            "failed.txt",
            DOCUMENT_FIXTURE.len(),
        ),
    );
    let telegram = telegram_server(updates).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetFile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": false,
            "error_code": 400,
            "description": "Bad Request: file is unavailable"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let failed = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Не удалось скачать файл. Попробуйте ещё раз."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), failed.wait_until_satisfied())
        .await
        .expect("download failure is reported");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(stored_file_count(&app.pool, assignment).await, 0);
}

#[tokio::test]
async fn published_attachment_is_deleted_only_after_telegram_confirmation() {
    use hw_bot::app::AddAssignmentFile;
    use hw_bot::domain::FilePresentation;

    let app = TestApp::start().await;
    let application = app.application.clone();
    let assignment = app.fixture.assignment.id();
    let card = application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), assignment)
        .await
        .unwrap();
    application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: assignment,
            actor: app.teacher_actor(),
            expected_revision: card.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
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
            &format!("manage:{}", assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("files:{}", assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), "file:delete:0", BOT_MESSAGE_ID),
        callback_update(7, app.teacher(), "file:confirm-delete", BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_file_count(&app.pool, assignment.as_uuid(), 0).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert!(requests.contains("Удалить вложение из уже выданного задания?"));
}

#[tokio::test]
async fn stale_file_callback_does_not_delete_the_attachment() {
    use hw_bot::app::AddAssignmentFile;
    use hw_bot::domain::FilePresentation;

    let app = TestApp::start().await;
    let assignment = app.fixture.assignment.id();
    let card = app
        .application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), assignment)
        .await
        .unwrap();
    app.application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: assignment,
            actor: app.teacher_actor(),
            expected_revision: card.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
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
            &format!("manage:{}", assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("files:{}", assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), "file:delete:0", BOT_MESSAGE_ID - 1),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied())
        .await
        .expect("stale file callback is rejected");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(stored_file_count(&app.pool, assignment.as_uuid()).await, 1);
}

#[tokio::test]
async fn student_receives_assignment_attachments_after_the_text() {
    use hw_bot::app::AddAssignmentFile;
    use hw_bot::domain::FilePresentation;

    let app = TestApp::start().await;
    let assignment = app.fixture.assignment.id();
    let initial = app
        .application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), assignment)
        .await
        .unwrap();
    let document = app
        .application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: assignment,
            actor: app.teacher_actor(),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
    app.application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: assignment,
            actor: app.teacher_actor(),
            expected_revision: document.revision,
            presentation: FilePresentation::Image,
            filename: "pixel.ppm".to_owned(),
            mime_type: Some("image/x-portable-pixmap".to_owned()),
            content: IMAGE_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(2, app.student(), "menu:assignments", BOT_MESSAGE_ID),
        callback_update(
            3,
            app.student(),
            &format!("assignment:{}", assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response())
        .expect(1)
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendPhoto"))
        .respond_with(telegram_photo_response())
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_request_path(&telegram, "/bottest-token/SendPhoto").await;
    stop_bot(cancellation, &mut task).await;
    telegram.verify().await;
    let requests = telegram.received_requests().await.unwrap();
    let body_message = requests
        .iter()
        .position(|request| {
            request.url.path() == "/bottest-token/SendMessage"
                && request
                    .body_json::<Value>()
                    .ok()
                    .and_then(|body| body["text"].as_str().map(str::to_owned))
                    == Some("Send a pull request link.".to_owned())
        })
        .expect("assignment body message");
    let document = requests
        .iter()
        .position(|request| request.url.path() == "/bottest-token/SendDocument")
        .expect("assignment document");
    let photo = requests
        .iter()
        .position(|request| request.url.path() == "/bottest-token/SendPhoto")
        .expect("assignment photo");

    assert!(body_message < document);
    assert!(document < photo);
    assert!(
        requests[document]
            .body
            .windows(DOCUMENT_FIXTURE.len())
            .any(|window| window == DOCUMENT_FIXTURE)
    );
    assert!(
        requests[photo]
            .body
            .windows(IMAGE_FIXTURE.len())
            .any(|window| window == IMAGE_FIXTURE)
    );
}

#[tokio::test]
async fn student_opens_a_previously_published_assignment_from_the_archive() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(2, app.student(), "menu:archive", BOT_MESSAGE_ID),
        callback_update(
            3,
            app.student(),
            &format!("assignment:{}", app.assignment()),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let opened = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "Send a pull request link."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), opened.wait_until_satisfied())
        .await
        .expect("archived assignment is opened");
    stop_bot(cancellation, &mut task).await;
    let callbacks = sent_callbacks(&telegram).await;

    assert!(callbacks.iter().any(|callback| callback == "menu:archive"));
    assert!(
        !callbacks
            .iter()
            .any(|callback| callback.starts_with("submit:"))
    );
}

#[tokio::test]
async fn teacher_changes_an_assignment_and_grants_attempts_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(14);
    let deadline_input = deadline_input(deadline);
    let expected_deadline = expected_deadline(&deadline_input);
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
            "text": "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu."
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

#[tokio::test]
async fn rate_limiter_silently_drops_updates_above_the_burst() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 1, Duration::ZERO).await;
    let settings = bot_settings_with(|settings| {
        settings.rate_limit_burst = 1;
        settings.rate_limit_replenish = Duration::from_secs(60);
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the admitted update is answered");
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn rate_limiter_has_an_independent_bucket_per_user() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, Duration::ZERO).await;
    let settings = bot_settings_with(|settings| {
        settings.rate_limit_burst = 1;
        settings.rate_limit_replenish = Duration::from_secs(60);
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("both users are answered");
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn concurrency_limit_drops_instead_of_queueing_updates() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 1, Duration::from_millis(250)).await;
    let settings = bot_settings_with(|settings| settings.max_concurrent_updates = 1);
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("one update is admitted");
    tokio::time::sleep(Duration::from_millis(350)).await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn updates_from_one_chat_are_processed_sequentially() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, Duration::from_millis(200)).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sent_message_count(&telegram).await, 1);
    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the second update runs after the first");
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn interactive_timeout_does_not_break_long_polling_or_queue_updates() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 1, Duration::from_millis(200)).await;
    let settings = bot_settings_with(|settings| {
        settings.request_timeout = Duration::from_millis(50);
        settings.max_concurrent_updates = 1;
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the admitted request reaches Telegram");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!task.is_finished(), "long polling must still be running");
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn telegram_rejects_oversized_input_before_registration() {
    let app = TestApp::start().await;
    let user_id = app.next_telegram_id();
    let token = app.group_invite_token().await;
    let updates = vec![
        message_update(1, user_id, 1, &format!("/start {token}")),
        message_update(2, user_id, 2, &"я".repeat(101)),
    ];
    let telegram = telegram_server(updates).await;
    let rejected = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": user_id,
            "text": "Имя не должно быть длиннее 100 символов."
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(2), rejected.wait_until_satisfied())
        .await
        .expect("oversized input is rejected");
    stop_bot(cancellation, &mut task).await;
    let exists = sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from users where telegram_user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn invalid_token_stops_the_telegram_bot() {
    let app = TestApp::start().await;
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false,
            "error_code": 401,
            "description": "Unauthorized"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, task) = start_bot(&app, &telegram);

    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("invalid token stops the bot")
        .expect("Telegram task does not panic");

    assert!(matches!(
        result,
        Err(hw_bot::bot::TelegramBotError::InvalidCredentials)
    ));
    cancellation.cancel();
}

fn start_bot(
    app: &TestApp,
    telegram: &MockServer,
) -> (
    CancellationToken,
    JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    start_bot_with_settings(app, telegram, bot_settings_with(|_| {}))
}

fn start_bot_with_settings(
    app: &TestApp,
    telegram: &MockServer,
    settings: TelegramBotSettings,
) -> (
    CancellationToken,
    JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
) {
    start_bot_with_application(telegram, app.application.clone(), settings)
}

fn start_bot_with_application(
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

fn file_upload_updates(app: &TestApp, assignment: Uuid, file: Value) -> Vec<Value> {
    vec![
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
            &format!("files:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(6, app.teacher(), "file:add", BOT_MESSAGE_ID),
        file,
    ]
}

fn bot_timezone() -> BotTimezone {
    BotTimezone::new(
        FixedOffset::east_opt(10 * 3600).unwrap(),
        NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
    )
}

fn deadline_input(deadline: DateTime<Utc>) -> String {
    deadline
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .format("%d.%m.%Y %H:%M")
        .to_string()
}

fn expected_deadline(input: &str) -> DateTime<Utc> {
    chrono::NaiveDateTime::parse_from_str(input, "%d.%m.%Y %H:%M")
        .unwrap()
        .and_local_timezone(FixedOffset::east_opt(10 * 3600).unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

fn bot_settings_with(update: impl FnOnce(&mut TelegramBotSettings)) -> TelegramBotSettings {
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

async fn stop_bot(
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
        .expect(2)
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
    for endpoint in ["SetMyCommands", "SetChatMenuButton"] {
        Mock::given(method("POST"))
            .and(path(format!("/bottest-token/{endpoint}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": true
            })))
            .mount(&telegram)
            .await;
    }

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

fn telegram_document_response() -> ResponseTemplate {
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

fn telegram_photo_response() -> ResponseTemplate {
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

async fn expect_sent_messages(
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

async fn wait_for_sent_message_count(telegram: &MockServer, expected: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if sent_message_count(telegram).await >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Telegram request is sent");
}

async fn sent_message_count(telegram: &MockServer) -> usize {
    telegram
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == "/bottest-token/SendMessage")
        .count()
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

fn document_update(
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

fn photo_update(update_id: u32, user_id: i64, message_id: i32) -> Value {
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

async fn mock_file_download(
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

async fn wait_for_request_path(telegram: &MockServer, expected: &str) {
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if telegram
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| request.url.path() == expected)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if result.is_err() {
        panic!(
            "Telegram request {expected} was not sent; requests:\n{}",
            recorded_requests(telegram).await
        );
    }
}

async fn wait_for_stored_file(pool: &PgPool, filename: &str) -> Option<(Vec<u8>, i64, String)> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = sqlx::query_as::<_, (Vec<u8>, i64, String)>(
                "select content, size_bytes, presentation from stored_files where filename = $1",
            )
            .bind(filename)
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

async fn wait_for_presentation(pool: &PgPool, presentation: &str) -> Option<(Vec<u8>, String)> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let row = sqlx::query_as::<_, (Vec<u8>, String)>(
                "select content, filename from stored_files where presentation = $1",
            )
            .bind(presentation)
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

async fn wait_for_file_count(pool: &PgPool, assignment_id: Uuid, expected: i64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let count: i64 =
                sqlx::query_scalar("select count(*) from stored_files where assignment_id = $1")
                    .bind(assignment_id)
                    .fetch_one(pool)
                    .await
                    .unwrap();
            if count == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("stored file count reaches expected value");
}

async fn stored_file_count(pool: &PgPool, assignment_id: Uuid) -> i64 {
    sqlx::query_scalar("select count(*) from stored_files where assignment_id = $1")
        .bind(assignment_id)
        .fetch_one(pool)
        .await
        .unwrap()
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

#[tokio::test]
async fn an_empty_review_queue_answers_the_teacher_instead_of_failing() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        message_update(2, app.teacher(), 2, "✅ Проверить работы"),
    ];
    let telegram = telegram_server(updates).await;
    let empty = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "🎉 Очередь проверки пуста.",
            "reply_markup": {
                "inline_keyboard": [
                    [{ "text": "✅ Проверить работы (0)", "callback_data": "review:next" }],
                    [{ "text": "📘 Мои курсы", "callback_data": "menu:courses" }],
                    [{ "text": "👤 Профиль", "callback_data": "menu:profile" }]
                ]
            }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), empty.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the teacher got no answer; requests:\n{requests}"));
}

#[tokio::test]
async fn the_first_start_in_a_fresh_chat_answers_with_the_menu() {
    let app = TestApp::start().await;
    let updates = vec![message_update(1, app.student(), 1, "/start")];
    let telegram = telegram_server(updates).await;
    let greeting = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "👋 Здравствуйте, Student!\nВыберите пункт меню ниже 👇",
            "reply_markup": {
                "keyboard": [
                    [{ "text": "📚 Мои задания" }, { "text": "⏳ Очередь проверки" }],
                    [{ "text": "🗄 Архив" }, { "text": "👤 Профиль" }],
                    [{ "text": "❓ Помощь" }]
                ],
                "is_persistent": true,
                "resize_keyboard": true,
                "input_field_placeholder": "Выберите пункт меню"
            }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), greeting.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("/start did not answer; requests:\n{requests}"));
}

#[tokio::test]
async fn navigation_from_an_older_message_still_opens_the_screen() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), "menu:courses", BOT_MESSAGE_ID),
        // Экран курса открыт с более нового сообщения, а жмём кнопки со старого.
        callback_update(
            3,
            app.teacher(),
            &format!("course:{}", app.course()),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("manage:{assignment}"),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("attempts:{assignment}"),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            6,
            app.teacher(),
            &format!("files:{assignment}"),
            BOT_MESSAGE_ID - 5,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu."
        })))
        .respond_with(telegram_message_response())
        .expect(0)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_request_path(&telegram, "/bottest-token/SendMessage").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    drop(stale);

    for expected in ["Выберите задание или создайте новое:", "Выберите студента:"]
    {
        assert!(
            requests.contains(expected),
            "{expected} was never shown; requests:\n{requests}"
        );
    }
    assert!(
        !requests.contains("старого сообщения"),
        "navigation must not be refused; requests:\n{requests}"
    );
}

/// А вот мутация со старой клавиатуры по-прежнему не проходит: один тап меняет задание.
#[tokio::test]
async fn a_mutation_from_an_older_message_is_still_refused() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("manage:{assignment}"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            3,
            app.teacher(),
            &format!("archive:{assignment}"),
            BOT_MESSAGE_ID - 5,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu."
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
    observed.unwrap_or_else(|_| panic!("stale mutation was not refused; requests:\n{requests}"));

    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(assignment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "published", "the assignment must not be archived");
}

const NEEDS_INVITE: &str = "Регистрация только по ссылке-приглашению группы. Попросите её у \
                            преподавателя и откройте присланную ссылку.";
const INVITE_INVALID: &str = "Приглашение недействительно. Попросите у преподавателя новую ссылку.";

#[tokio::test]
async fn start_without_an_invite_never_offers_a_group() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = telegram_server(vec![message_update(1, guest, 1, "/start")]).await;
    let refused = expect_text(&telegram, guest, NEEDS_INVITE).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), refused.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("guest was not asked for an invite; requests:\n{requests}"));
    assert!(
        !requests.contains(&app.fixture.group.code().as_str().to_owned()),
        "the guest must not see any group code; requests:\n{requests}"
    );
}

#[tokio::test]
async fn a_valid_link_names_the_group_and_registers_the_student() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let token = app.group_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, guest, 1, &format!("/start {token}")),
        message_update(2, guest, 2, "Ivan"),
    ])
    .await;
    let prompt = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": guest,
            "text": format!(
                "Регистрация в группу {}.\n\nКак вас называть?\n\n/cancel — отменить",
                app.fixture.group.code().as_str()
            ),
            "reply_markup": { "remove_keyboard": true }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let asked = tokio::time::timeout(Duration::from_secs(5), prompt.wait_until_satisfied()).await;
    let registered = wait_for_group(&app.pool, guest).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    asked.unwrap_or_else(|_| panic!("the group was not named; requests:\n{requests}"));
    assert_eq!(registered, Some(app.fixture.group.id().as_uuid()));
}

/// Битая подпись и уже отозванная ссылка обязаны выглядеть для гостя одинаково.
#[tokio::test]
async fn a_forged_and_a_rotated_link_answer_the_same() {
    let app = TestApp::start().await;
    let forged_user = app.next_telegram_id();
    let stale_user = app.next_telegram_id();
    let stale_token = app.group_invite_token().await;
    hw_bot::db::group_invites::rotate(
        &app.pool,
        app.fixture.group.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    let telegram = telegram_server(vec![
        message_update(
            1,
            forged_user,
            1,
            "/start gi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ),
        message_update(2, stale_user, 2, &format!("/start {stale_token}")),
    ])
    .await;
    let forged = expect_text(&telegram, forged_user, INVITE_INVALID).await;
    let stale = expect_text(&telegram, stale_user, INVITE_INVALID).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let forged = tokio::time::timeout(Duration::from_secs(5), forged.wait_until_satisfied()).await;
    let stale = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    forged.unwrap_or_else(|_| panic!("forged link answered differently; requests:\n{requests}"));
    stale.unwrap_or_else(|_| panic!("rotated link answered differently; requests:\n{requests}"));
}

#[tokio::test]
async fn a_registered_student_keeps_their_group_when_opening_a_foreign_link() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let token = app.invite_token(foreign.group.id()).await;
    let telegram = telegram_server(vec![message_update(
        1,
        app.student(),
        1,
        &format!("/start {token}"),
    )])
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 1).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(
        wait_for_group(&app.pool, app.student()).await,
        Some(app.fixture.group.id().as_uuid()),
        "an invite must not move an existing student"
    );
}

#[tokio::test]
async fn teacher_gets_a_forwardable_link_for_their_course() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("invite:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let link = expect_text(
        &telegram,
        app.teacher(),
        &format!("https://t.me/test_bot?start={token}"),
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), link.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the link was not sent; requests:\n{requests}"));
}

#[tokio::test]
async fn a_teacher_of_another_course_gets_no_link_from_a_replayable_button() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let outsider = foreign.teacher.telegram_user_id().as_i64();
    let telegram = telegram_server(vec![
        message_update(1, outsider, 1, "/menu"),
        callback_update(
            2,
            outsider,
            &format!("invite:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let denied = expect_text(&telegram, outsider, "Это действие вам недоступно.").await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), denied.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed
        .unwrap_or_else(|_| panic!("the foreign course was not refused; requests:\n{requests}"));
    assert!(
        !requests.contains("https://t.me/test_bot?start="),
        "no link may leak; requests:\n{requests}"
    );
}

#[tokio::test]
async fn a_confirmed_rotation_replaces_the_link() {
    let app = TestApp::start().await;
    let old_token = app.group_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("invite:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(3, app.teacher(), "invite:rotate", BOT_MESSAGE_ID),
        callback_update(4, app.teacher(), "invite:confirm-rotate", BOT_MESSAGE_ID),
    ])
    .await;
    let replaced = expect_text(
        &telegram,
        app.teacher(),
        "♻️ Прежняя ссылка больше не работает.",
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), replaced.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("rotation did not happen; requests:\n{requests}"));

    let new_token = app.group_invite_token().await;
    assert_ne!(new_token, old_token);
    app.register_student(app.next_telegram_id(), "Late", &old_token)
        .await
        .error(axum::http::StatusCode::FORBIDDEN, "invite_invalid");
    app.register_student(app.next_telegram_id(), "Ivan", &new_token)
        .await
        .created();
}

#[tokio::test]
async fn a_stale_confirm_button_does_not_rotate_twice() {
    let app = TestApp::start().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("invite:{}", app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(3, app.teacher(), "invite:rotate", BOT_MESSAGE_ID),
        callback_update(4, app.teacher(), "invite:confirm-rotate", BOT_MESSAGE_ID),
        callback_update(5, app.teacher(), "invite:confirm-rotate", BOT_MESSAGE_ID),
    ])
    .await;
    let stale = expect_text(
        &telegram,
        app.teacher(),
        "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu.",
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the stale confirm was accepted; requests:\n{requests}"));

    let generation: i64 = sqlx::query_scalar("select generation from student_group_invites")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(generation, 2, "only the first confirmation may rotate");
}

async fn expect_text(telegram: &MockServer, chat_id: i64, text: &str) -> wiremock::MockGuard {
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({ "chat_id": chat_id, "text": text })))
        .respond_with(telegram_message_response())
        .expect(1..)
        .with_priority(1)
        .mount_as_scoped(telegram)
        .await
}

async fn wait_for_group(pool: &PgPool, telegram_user_id: i64) -> Option<Uuid> {
    for _ in 0..50 {
        let group: Option<Option<Uuid>> =
            sqlx::query_scalar("select student_group_id from users where telegram_user_id = $1")
                .bind(telegram_user_id)
                .fetch_optional(pool)
                .await
                .unwrap();
        if let Some(group) = group {
            return group;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    None
}
