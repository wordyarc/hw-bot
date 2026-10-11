use std::time::Duration;

use chrono::Utc;
use hw_bot::app::{ApplicationContainer, ErrorCode};
use hw_bot::bot::{app_error_message, callbacks, texts};
use serde_json::{Value, json};
use wiremock::matchers::{body_json, body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::harness::telegram::*;
use crate::harness::{TestApp, test_file_policy};

#[tokio::test]
async fn teacher_adds_and_opens_a_real_document_during_draft_creation() {
    let app = TestApp::start().await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(7));
    let updates = vec![
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
            &format!("{}{}", callbacks::CREATE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
        message_update(5, app.teacher(), 2, "Draft with attachment"),
        message_update(6, app.teacher(), 3, "Read the attached notes."),
        message_update(7, app.teacher(), 4, "2"),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, "passfail"),
            BOT_MESSAGE_ID,
        ),
        message_update(9, app.teacher(), 5, &deadline),
        callback_update(10, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
        callback_update(11, app.teacher(), callbacks::FILE_ADD, BOT_MESSAGE_ID),
        document_update(
            12,
            app.teacher(),
            6,
            "fixture-document",
            "assignment-note.txt",
            DOCUMENT_FIXTURE.len(),
        ),
        callback_update(
            13,
            app.teacher(),
            &cb(callbacks::FILE_OPEN_PREFIX, 0),
            BOT_MESSAGE_ID,
        ),
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
    let resend = requests
        .iter()
        .find(|request| request.url.path() == "/bottest-token/SendDocument")
        .expect("SendDocument request");
    assert!(contains(&resend.body, b"fixture-document"));
    assert!(!contains(&resend.body, DOCUMENT_FIXTURE));
}

#[tokio::test]
async fn teacher_adds_and_opens_the_largest_photo_variant() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = vec![
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
        photo_update(7, app.teacher(), 2),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::FILE_OPEN_PREFIX, 0),
            BOT_MESSAGE_ID,
        ),
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
        .and(body_partial_json(json!({
            "chat_id": app.teacher(),
            "text": app_error_message(ErrorCode::FileTooLarge)
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
    let rejected = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": app_error_message(ErrorCode::FileTooLarge)
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

    tokio::time::timeout(Duration::from_secs(5), rejected.wait_until_satisfied())
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
            "text": texts::FILE_DOWNLOAD_FAILED
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
            telegram_file_id: None,
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/courses");
    for data in [
        cb(callbacks::COURSE_PREFIX, app.course()),
        cb(callbacks::MANAGE_PREFIX, assignment.as_uuid()),
        cb(callbacks::FILES_PREFIX, assignment.as_uuid()),
        cb(callbacks::FILE_DELETE_PREFIX, 0),
    ] {
        telegram.click_button(app.teacher(), &data).await;
    }
    let confirm = telegram
        .button(app.teacher(), callbacks::FILE_CONFIRM_DELETE)
        .await;
    assert_eq!(stored_file_count(&app.pool, assignment.as_uuid()).await, 1);

    telegram.click(app.teacher(), &confirm);
    telegram.flush(app.teacher()).await;
    assert_eq!(stored_file_count(&app.pool, assignment.as_uuid()).await, 0);
    stop_bot(cancellation, &mut task).await;
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
            telegram_file_id: None,
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
    let updates = vec![
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
            &format!("{}{}", callbacks::MANAGE_PREFIX, assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("{}{}", callbacks::FILES_PREFIX, assignment.as_uuid()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            6,
            app.teacher(),
            &cb(callbacks::FILE_DELETE_PREFIX, 0),
            BOT_MESSAGE_ID - 1,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": texts::STALE_BUTTON
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
            telegram_file_id: None,
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
            telegram_file_id: None,
            content: IMAGE_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            callbacks::MENU_ASSIGNMENTS,
            BOT_MESSAGE_ID,
        ),
        callback_update(
            3,
            app.student(),
            &format!("{}{}", callbacks::ASSIGNMENT_PREFIX, assignment.as_uuid()),
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
async fn an_uploaded_attachment_is_resent_by_its_telegram_file_id() {
    let app = TestApp::start().await;
    add_document(&app, None).await;
    let open = cb(callbacks::ASSIGNMENT_PREFIX, app.assignment());
    let telegram = telegram_server(vec![
        callback_update(1, app.student(), &open, BOT_MESSAGE_ID),
        callback_update(2, app.student(), &open, BOT_MESSAGE_ID),
    ])
    .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response())
        .expect(2)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_request_count(&telegram, "/bottest-token/SendDocument", 2).await;
    stop_bot(cancellation, &mut task).await;

    let documents = sent_documents(&telegram).await;
    assert!(contains(&documents[0], DOCUMENT_FIXTURE));
    assert!(contains(&documents[1], b"sent-document"));
    assert!(!contains(&documents[1], DOCUMENT_FIXTURE));
}

#[tokio::test]
async fn a_rejected_telegram_file_id_falls_back_to_the_stored_bytes() {
    let app = TestApp::start().await;
    add_document(&app, Some("foreign-file")).await;
    let telegram = telegram_server(vec![callback_update(
        1,
        app.student(),
        &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        BOT_MESSAGE_ID,
    )])
    .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .and(|request: &wiremock::Request| contains(&request.body, b"foreign-file"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false,
            "error_code": 400,
            "description": "Bad Request: wrong file identifier/HTTP URL specified"
        })))
        .expect(1)
        .with_priority(1)
        .mount(&telegram)
        .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response())
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let replaced = wait_until(Duration::from_secs(5), || async {
        stored_telegram_file_id(&app).await.as_deref() == Some("sent-document")
    })
    .await;
    stop_bot(cancellation, &mut task).await;

    assert!(replaced, "the uploaded copy replaced the rejected file id");
    let documents = sent_documents(&telegram).await;
    assert!(contains(&documents[1], DOCUMENT_FIXTURE));
}

#[tokio::test]
async fn a_resend_is_not_repeated_as_an_upload_unless_the_file_id_was_rejected() {
    let app = TestApp::start().await;
    add_document(&app, Some("limited-file")).await;
    let telegram = telegram_server(vec![callback_update(
        1,
        app.student(),
        &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        BOT_MESSAGE_ID,
    )])
    .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false,
            "error_code": 429,
            "description": "Too Many Requests: retry after 5",
            "parameters": {"retry_after": 5}
        })))
        .mount(&telegram)
        .await;
    let failed = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "chat_id": app.student(),
            "text": texts::ASSIGNMENT_FILES_SEND_FAILED
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), failed.wait_until_satisfied())
        .await
        .expect("the failed delivery is reported");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(sent_documents(&telegram).await.len(), 1);
}

#[tokio::test]
async fn a_file_the_teacher_opens_without_success_is_reported() {
    let app = TestApp::start().await;
    add_document(&app, Some("limited-file")).await;
    let mut updates = file_upload_updates(&app, app.assignment(), json!(null));
    updates.truncate(5);
    updates.push(callback_update(
        6,
        app.teacher(),
        &cb(callbacks::FILE_OPEN_PREFIX, 0),
        BOT_MESSAGE_ID,
    ));
    let telegram = telegram_server(updates).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false,
            "error_code": 429,
            "description": "Too Many Requests: retry after 5",
            "parameters": {"retry_after": 5}
        })))
        .mount(&telegram)
        .await;
    let failed = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "chat_id": app.teacher(),
            "text": texts::FILE_SEND_FAILED
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(5), failed.wait_until_satisfied())
        .await
        .expect("the failed delivery is reported");
    stop_bot(cancellation, &mut task).await;
}

async fn add_document(app: &TestApp, telegram_file_id: Option<&str>) {
    use hw_bot::app::AddAssignmentFile;
    use hw_bot::domain::FilePresentation;

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
            telegram_file_id: telegram_file_id.map(str::to_owned),
            content: DOCUMENT_FIXTURE.to_vec(),
        })
        .await
        .unwrap();
}

async fn stored_telegram_file_id(app: &TestApp) -> Option<String> {
    sqlx::query_scalar("select telegram_file_id from stored_files where assignment_id = $1")
        .bind(app.assignment())
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn sent_documents(telegram: &wiremock::MockServer) -> Vec<Vec<u8>> {
    telegram
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/bottest-token/SendDocument")
        .map(|request| request.body)
        .collect()
}

fn contains(body: &[u8], needle: &[u8]) -> bool {
    body.windows(needle.len()).any(|window| window == needle)
}
