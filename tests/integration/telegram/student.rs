use std::time::Duration;

use chrono::{DateTime, Utc};
use hw_bot::app::ErrorCode;
use hw_bot::bot::{app_error_message, callbacks, texts};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, DeductedFrom, Submission, SubmissionId, SubmissionKind, SubmissionStatus,
    USER_TEXT_MAX_LEN,
};
use serde_json::json;
use wiremock::matchers::{body_json, body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{
    Button, TelegramScenario, text_over_the_limit_once_its_link_is_revealed,
};
use crate::harness::{TestApp, insert_assignment, out_of, submission_id, wall_clock_assignment};

async fn delayed_submission(
    app: &TestApp,
    assignment: &Assignment,
    sent_at: DateTime<Utc>,
) -> Submission {
    let mut submission = message_update(2, app.student(), 2, "pull/on-time");
    submission["message"]["date"] = json!(sent_at.timestamp());
    let telegram = telegram_server(vec![
        callback_update(
            1,
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, assignment.id().as_uuid()),
            BOT_MESSAGE_ID,
        ),
        submission,
    ])
    .await;
    let (cancellation, mut task) = start_bot(app, &telegram);
    wait_for_sent_message_count(&telegram, 2).await;
    stop_bot(cancellation, &mut task).await;
    db::submissions::latest_for_student(&app.pool, assignment.id(), app.fixture.student.id())
        .await
        .unwrap()
        .expect("a submission sent before closing must be stored")
}

#[tokio::test]
async fn a_delayed_on_time_submission_uses_the_original_message_date() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(0, None).await;
    let sent_at = DateTime::from_timestamp(assignment.deadline_at().timestamp() - 1, 0).unwrap();

    let submission = delayed_submission(&app, &assignment, sent_at).await;

    assert_eq!(submission.submitted_at(), sent_at);
    assert!(submission.is_pending_review());
}

#[tokio::test]
async fn student_opens_a_previously_published_assignment_from_the_archive() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(2, app.student(), callbacks::MENU_ARCHIVE, BOT_MESSAGE_ID),
        callback_update(
            3,
            app.student(),
            &format!("{}{}", callbacks::ASSIGNMENT_PREFIX, app.assignment()),
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

    assert!(
        callbacks
            .iter()
            .any(|callback| callback == callbacks::MENU_ARCHIVE)
    );
    assert!(
        !callbacks
            .iter()
            .any(|callback| callback.starts_with(callbacks::SUBMIT_PREFIX))
    );
}

#[tokio::test]
async fn the_history_shows_a_penalty_to_the_course_total_next_to_the_work_score() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(submission, app.teacher(), "accept", out_of(8, 10), "Ок")
        .await
        .ok();
    let grade = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap()
        .grade()
        .unwrap();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    let history = telegram
        .button_with_text(app.student(), texts::BUTTON_HISTORY)
        .await;
    telegram.click(app.student(), &history);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let sent: Vec<String> = telegram
        .messages(app.student())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    let header = texts::history_header(
        app.fixture.subject.name().as_str(),
        assignment.title().as_str(),
    );
    let opened = sent
        .iter()
        .position(|text| *text == header)
        .unwrap_or_else(|| panic!("the history was not opened: {sent:#?}"));
    let attempt = texts::attempt_line(
        1,
        texts::submission_status(SubmissionStatus::Accepted),
        texts::grade_line(SubmissionKind::Regular, Some(grade)).as_deref(),
    );
    assert!(
        sent[opened..].iter().any(|text| text.starts_with(&attempt)),
        "{sent:#?}"
    );
    assert_eq!(grade.deducted_from(), Some(DeductedFrom::CourseTotal));
    assert_eq!((grade.work_score(), grade.course_total_penalty()), (8, 4));
}

#[tokio::test]
async fn an_old_submit_button_names_why_the_work_cannot_be_submitted_instead_of_asking_for_it() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        )
        .await;
    let submit = telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    telegram.message(app.student(), "pull/1");
    telegram.flush(app.student()).await;
    telegram.click(app.student(), &submit);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let sent: Vec<String> = telegram
        .messages(app.student())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    let prompts = sent
        .iter()
        .filter(|text| **text == texts::input_prompt(texts::SUBMISSION_PROMPT))
        .count();
    assert_eq!(prompts, 1, "{sent:#?}");
    assert!(
        sent.contains(&app_error_message(ErrorCode::SubmissionPendingReview).to_owned()),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn a_second_submit_tap_refused_after_the_prompt_brings_the_menu_back() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        )
        .await;
    let submit = telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    let before = telegram.messages(app.student()).len();
    telegram.click(app.student(), &submit);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let replies = &telegram.messages(app.student())[before..];
    assert!(
        replies
            .iter()
            .any(|message| message["reply_markup"]["keyboard"].is_array()),
        "{replies:#?}"
    );
}

#[tokio::test]
async fn a_submission_refused_after_the_prompt_brings_the_menu_back() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        )
        .await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    telegram.message(app.student(), "pull/1");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let refusal = telegram
        .messages(app.student())
        .into_iter()
        .find(|message| message["text"] == app_error_message(ErrorCode::AssignmentUnavailable))
        .expect("the refusal is sent");
    assert!(
        refusal["reply_markup"]["keyboard"].is_array(),
        "{refusal:#?}"
    );
}

#[tokio::test]
async fn the_assignment_card_names_a_penalty_to_the_course_total() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    stop_bot(cancellation, &mut task).await;

    let deducted_from = texts::deducted_from_line(DeductedFrom::CourseTotal);
    let sent: Vec<String> = telegram
        .messages(app.student())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    let card = sent
        .iter()
        .find(|text| text.contains(&deducted_from))
        .unwrap_or_else(|| panic!("the card does not name the deducted_from: {sent:#?}"));
    assert!(card.contains(assignment.title().as_str()), "{card}");
    assert!(
        card.contains(&format!("{} −4", texts::LATE_PENALTY_LABEL)),
        "{card}"
    );
}

fn longer_than_one_telegram_message() -> String {
    format!("{}{}", "📌".repeat(1000), "a".repeat(2100))
}

async fn open_submission_prompt(app: &TestApp, telegram: &TelegramScenario) -> Button {
    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        )
        .await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await
}

#[tokio::test]
async fn a_file_on_the_submission_step_is_refused_and_the_step_waits_for_text() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_submission_prompt(&app, &telegram).await;

    telegram.document(
        app.student(),
        "work-file",
        "work.txt",
        DOCUMENT_FIXTURE.len(),
    );
    let refused = wait_until(Duration::from_secs(5), || async {
        telegram
            .texts(app.student())
            .contains(&texts::TEXT_REQUIRED.to_owned())
    })
    .await;
    let after_file = latest_submission(&app).await;
    telegram.message(app.student(), "pull/after-file");
    let after_text = wait_for_some(Duration::from_secs(5), || latest_submission(&app)).await;
    stop_bot(cancellation, &mut task).await;

    assert!(refused, "{:#?}", telegram.texts(app.student()));
    assert!(after_file.is_none());
    assert_eq!(
        after_text
            .expect("the text after the file is submitted")
            .content()
            .as_str(),
        "pull/after-file"
    );
}

async fn latest_submission(app: &TestApp) -> Option<Submission> {
    db::submissions::latest_for_student(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_body_longer_than_one_telegram_message_still_brings_the_card_buttons() {
    let app = TestApp::start().await;
    let assignment = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
        .title("Long body")
        .body(longer_than_one_telegram_message())
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    telegram
        .button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_submission_longer_than_one_telegram_message_once_its_link_is_revealed_keeps_the_card_buttons()
 {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_submission_prompt(&app, &telegram).await;
    telegram.message_with(
        app.student(),
        json!({
            "text": longer_than_one_telegram_message(),
            "entities": [{
                "type": "text_link",
                "offset": 2000,
                "length": 10,
                "url": "https://example.test/a-hidden-link"
            }]
        }),
    );
    telegram.flush(app.student()).await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        )
        .await;
    telegram
        .button_with_text(app.student(), texts::BUTTON_HISTORY)
        .await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_submission_over_the_limit_once_its_link_is_revealed_is_refused_and_the_step_stays() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_submission_prompt(&app, &telegram).await;
    telegram.message_with(
        app.student(),
        text_over_the_limit_once_its_link_is_revealed(),
    );
    telegram.message(app.student(), "pull/1");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let refusal = texts::text_with_links_too_long(texts::FIELD_SUBMISSION, USER_TEXT_MAX_LEN);
    assert!(
        telegram
            .messages(app.student())
            .iter()
            .any(|message| message["text"] == refusal.as_str()),
        "the refusal names the length"
    );
    let stored = db::submissions::latest_for_student(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await
    .unwrap()
    .expect("the shorter text is taken by the same step");
    assert_eq!(stored.content().as_str(), "pull/1");
}

#[tokio::test]
async fn a_second_submit_tap_that_fails_on_the_database_brings_the_menu_back() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    let submit = open_submission_prompt(&app, &telegram).await;
    let prompt = texts::input_prompt(texts::SUBMISSION_PROMPT);
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(app.student())
            .into_iter()
            .find(|message| message["text"] == prompt.as_str())
    })
    .await
    .expect("the submission prompt is sent");
    let before = telegram.messages(app.student()).len();
    app.pool.close().await;
    telegram.click(app.student(), &submit);
    let failure = app_error_message(ErrorCode::InternalError);
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(app.student())
            .into_iter()
            .find(|message| message["text"] == failure)
    })
    .await
    .expect("the failure is reported");
    stop_bot(cancellation, &mut task).await;

    let replies = &telegram.messages(app.student())[before..];
    assert!(
        replies
            .iter()
            .any(|message| message["reply_markup"]["keyboard"].is_array()),
        "{replies:#?}"
    );
}

#[tokio::test]
async fn a_second_submit_tap_whose_prompt_fails_to_send_does_not_announce_a_cancel() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    let submit = open_submission_prompt(&app, &telegram).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(
            json!({"text": texts::input_prompt(texts::SUBMISSION_PROMPT)}),
        ))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false,
            "error_code": 400,
            "description": "Bad Request: the prompt was not sent"
        })))
        .with_priority(1)
        .up_to_n_times(1)
        .mount(&telegram.server)
        .await;
    let before = telegram.messages(app.student()).len();
    telegram.click(app.student(), &submit);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let replies = &telegram.messages(app.student())[before..];
    assert!(
        !replies
            .iter()
            .any(|message| message["text"] == texts::ACTION_CANCELLED),
        "{replies:#?}"
    );
}
