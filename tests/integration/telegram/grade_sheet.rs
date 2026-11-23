use hw_bot::app::ErrorCode;
use hw_bot::bot::{app_error_message, callbacks, texts};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use super::flows::{open_course, open_course_of};
use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{Button, TelegramScenario};

const SEND_DOCUMENT: &str = "/bottest-token/SendDocument";
const XLSX_SIGNATURE: &[u8] = b"PK\x03\x04";

async fn mount_document(telegram: &TelegramScenario) {
    Mock::given(method("POST"))
        .and(path(SEND_DOCUMENT))
        .respond_with(telegram_document_response())
        .with_priority(10)
        .mount(&telegram.server)
        .await;
}

async fn documents(telegram: &TelegramScenario) -> Vec<Vec<u8>> {
    telegram
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == SEND_DOCUMENT)
        .map(|request| request.body)
        .collect()
}

fn contains(body: &[u8], needle: &[u8]) -> bool {
    body.windows(needle.len()).any(|window| window == needle)
}

fn received_text(telegram: &TelegramScenario, user: i64, expected: &str) -> bool {
    telegram
        .messages(user)
        .iter()
        .any(|message| message["text"] == expected)
}

async fn sheet_button(app: &TestApp, telegram: &TelegramScenario) -> Button {
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_GRADE_SHEET)
        .await
}

async fn open_terms_form(
    app: &TestApp,
    telegram: &TelegramScenario,
    title: &str,
) -> (Button, String) {
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), title);
    telegram.message(app.teacher(), "Keep this body");
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 3))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADING_PREFIX, 5))
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    let deadline = deadline_input(chrono::Utc::now() + chrono::Duration::days(7));
    telegram.message(app.teacher(), &deadline);
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 2))
        .await;
    let periods = telegram
        .button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;

    (periods, deadline)
}

async fn finish_terms_form(
    app: &TestApp,
    telegram: &TelegramScenario,
    periods: &Button,
    title: &str,
    deadline: &str,
) {
    telegram.click(app.teacher(), periods);
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::FILE_FINISH)
        .await;

    let created = wait_for_created_assignment(&app.pool, title).await.unwrap();
    assert_eq!(created.body, "Keep this body");
    assert_eq!(created.max_attempts, 3);
    assert_eq!(created.max_score, Some(5));
    assert_eq!(created.deadline_at, expected_deadline(deadline));
}

#[tokio::test]
async fn the_button_of_the_subject_screen_sends_the_sheet_as_an_xlsx_document() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    mount_document(&telegram).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_course(&app, &telegram).await;
    let button = sheet_button(&app, &telegram).await;
    telegram.click(app.teacher(), &button);
    wait_for_request_path(&telegram.server, SEND_DOCUMENT).await;
    stop_bot(cancellation, &mut task).await;

    let documents = documents(&telegram).await;
    assert_eq!(documents.len(), 1);
    assert!(contains(&documents[0], "Ведомость".as_bytes()));
    assert!(contains(&documents[0], b".xlsx"));
    assert!(contains(&documents[0], XLSX_SIGNATURE));
}

#[tokio::test]
async fn a_closed_course_still_offers_the_sheet() {
    let app = TestApp::start().await;
    app.mark_course_closed(&app.fixture.course).await;
    let telegram = TelegramScenario::new().await;
    mount_document(&telegram).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/courses");
    let closed = telegram
        .button_with_text(app.teacher(), texts::BUTTON_CLOSED_COURSES)
        .await;
    telegram.click(app.teacher(), &closed);
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    let button = sheet_button(&app, &telegram).await;
    telegram.click(app.teacher(), &button);
    wait_for_request_path(&telegram.server, SEND_DOCUMENT).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(documents(&telegram).await.len(), 1);
}

#[tokio::test]
async fn a_course_without_students_gets_a_message_instead_of_a_file() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    app.attach_course_teacher(&foreign.course, &app.fixture.teacher)
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_course_of(&app, &telegram, foreign.course.id().as_uuid()).await;
    let button = sheet_button(&app, &telegram).await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(received_text(
        &telegram,
        app.teacher(),
        texts::GRADE_SHEET_NO_STUDENTS
    ));
    assert!(documents(&telegram).await.is_empty());
}

#[tokio::test]
async fn a_teacher_removed_from_the_course_gets_a_refusal_from_the_old_button() {
    let app = TestApp::start().await;
    app.add_course_teacher("Remaining teacher").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_course(&app, &telegram).await;
    let button = sheet_button(&app, &telegram).await;
    app.remove_course_teacher(&app.fixture.teacher).await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(received_text(
        &telegram,
        app.teacher(),
        app_error_message(ErrorCode::PermissionDenied)
    ));
    assert!(documents(&telegram).await.is_empty());
}

#[tokio::test]
async fn a_student_pressing_a_forged_button_gets_a_refusal_and_no_file() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_course(&app, &telegram).await;
    let button = sheet_button(&app, &telegram).await;
    telegram.click(app.student(), &button.in_chat(app.student()));
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(received_text(
        &telegram,
        app.student(),
        app_error_message(ErrorCode::PermissionDenied)
    ));
    assert!(documents(&telegram).await.is_empty());
}

#[tokio::test]
async fn pressing_the_button_during_a_form_keeps_its_step_and_values() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    mount_document(&telegram).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_course(&app, &telegram).await;
    let button = sheet_button(&app, &telegram).await;
    let (periods, deadline) = open_terms_form(&app, &telegram, "Sheet-safe draft").await;
    let original = telegram.message_by_id(app.teacher(), periods.message_id);

    telegram.click(app.teacher(), &button);
    wait_for_request_path(&telegram.server, SEND_DOCUMENT).await;
    telegram.flush(app.teacher()).await;

    assert_eq!(
        telegram.message_by_id(app.teacher(), periods.message_id),
        original
    );
    finish_terms_form(&app, &telegram, &periods, "Sheet-safe draft", &deadline).await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_failed_send_during_a_form_reports_an_error_and_keeps_its_step() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    Mock::given(method("POST"))
        .and(path(SEND_DOCUMENT))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false,
            "error_code": 400,
            "description": "Bad Request: file is rejected"
        })))
        .expect(1)
        .with_priority(10)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_course(&app, &telegram).await;
    let button = sheet_button(&app, &telegram).await;
    let (periods, deadline) = open_terms_form(&app, &telegram, "Failed sheet draft").await;

    telegram.click(app.teacher(), &button);
    wait_for_request_path(&telegram.server, SEND_DOCUMENT).await;
    telegram.flush(app.teacher()).await;

    assert!(received_text(
        &telegram,
        app.teacher(),
        app_error_message(ErrorCode::InternalError)
    ));
    finish_terms_form(&app, &telegram, &periods, "Failed sheet draft", &deadline).await;
    stop_bot(cancellation, &mut task).await;
    telegram.server.verify().await;
}
