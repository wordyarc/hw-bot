use std::time::Duration;

use chrono::{DateTime, Utc};
use hw_bot::bot::{Clock, TelegramBot, TelegramBotError, callbacks, texts};
use hw_bot::domain::FilePresentation;
use hw_bot::health::{CriticalComponent, HealthState};
use hw_bot::supervision::{CriticalTask, Supervisor};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use hw_bot::app::{AddAssignmentFile, ErrorCode};
use hw_bot::bot::app_error_message;

use super::flows::{manage_assignment, open_assignment, open_course, open_submission_prompt};
use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{Button, TelegramScenario};

#[tokio::test]
async fn a_command_reports_an_application_error_to_the_user_once() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.student(), "/courses");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        telegram
            .messages(app.student())
            .iter()
            .filter(|message| message["text"] == app_error_message(ErrorCode::PermissionDenied))
            .count(),
        1
    );
}

fn offers_submission(telegram: &TelegramScenario, user: i64) -> bool {
    telegram.messages(user).iter().any(|message| {
        message["reply_markup"]["inline_keyboard"]
            .as_array()
            .is_some_and(|rows| {
                rows.iter()
                    .flat_map(|row| row.as_array().unwrap())
                    .any(|button| {
                        button["callback_data"].as_str().is_some_and(|data| {
                            data.starts_with(callbacks::SUBMIT_PREFIX)
                                || data.starts_with(callbacks::SUBMIT_MAKEUP_PREFIX)
                        })
                    })
            })
    })
}

#[tokio::test]
async fn the_submit_button_returns_only_after_the_pending_work_is_rejected() {
    let app = TestApp::start().await;
    let pending = app.queued_submission(app.student(), "pull/first").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_assignment(&app, &telegram, app.assignment()).await;
    assert!(!offers_submission(&telegram, app.student()));

    app.review(pending, app.teacher(), "reject", "Retry")
        .await
        .ok();
    open_assignment(&app, &telegram, app.assignment()).await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    telegram.message(app.student(), "pull/retry");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn a_refused_submission_closes_the_input_step() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_submission_prompt(&app, &telegram).await;
    // Шаг ввода уже открыт. Имитируем сдачу с другого устройства, чтобы следующая получила доменный отказ.
    app.queued_submission(app.student(), "pull/first").await;
    telegram.message(app.student(), "pull/retry");
    telegram.message(app.student(), "pull/again");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let messages = telegram.messages(app.student());
    let count = |text: &str| messages.iter().filter(|m| m["text"] == text).count();
    assert_eq!(
        count(app_error_message(ErrorCode::SubmissionPendingReview)),
        1
    );
    assert_eq!(count(texts::USE_THE_MENU), 1);
    let stored: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored, 1);
}

async fn fail_next(telegram: &TelegramScenario, endpoint: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/bottest-token/{endpoint}")))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400, "description": "Bad Request: injected failure",
        })))
        .up_to_n_times(1)
        .expect(1)
        .with_priority(1)
        .mount(&telegram.server)
        .await;
}

#[tokio::test]
async fn failed_terms_transition_keeps_grading_choice_retryable() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_course(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), "Recoverable draft");
    telegram.message(app.teacher(), "Solve the task");
    telegram.message(app.teacher(), "2");
    let grading_data = cb(callbacks::GRADING_PREFIX, "passfail");
    let grading = telegram.button(app.teacher(), &grading_data).await;
    Mock::given(method("POST"))
        .and(|request: &wiremock::Request| {
            matches!(
                request.url.path(),
                "/bottest-token/SendMessage" | "/bottest-token/EditMessageText"
            )
        })
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false, "error_code": 429,
            "description": "Too Many Requests: retry after 1",
            "parameters": {"retry_after": 1},
        })))
        .up_to_n_times(1)
        .expect(1)
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    telegram.click(app.teacher(), &grading);
    telegram.flush(app.teacher()).await;
    telegram.click_button(app.teacher(), &grading_data).await;
    telegram
        .click_preset(app.teacher(), texts::PRESET_WEEK)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let created = wait_for_created_assignment(&app.pool, "Recoverable draft")
        .await
        .expect("the same draft can be completed after retrying the transition");

    assert_eq!(created.max_score, None);
    assert_eq!(created.max_attempts, 2);
    assert_eq!(created.body, "Solve the task");
}

async fn open_terms(app: &TestApp, telegram: &TelegramScenario) -> Button {
    manage_assignment(app, telegram, app.assignment()).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::DEADLINE_PREFIX, app.assignment()),
        )
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await
}

async fn reject_message_edits(telegram: &TelegramScenario, message_id: i32, description: &str) {
    for endpoint in ["EditMessageText", "EditMessageReplyMarkup"] {
        Mock::given(method("POST"))
            .and(path(format!("/bottest-token/{endpoint}")))
            .and(body_partial_json(json!({"message_id": message_id})))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "ok": false, "error_code": 400, "description": description,
            })))
            .with_priority(1)
            .mount(&telegram.server)
            .await;
    }
}

#[tokio::test]
async fn unavailable_terms_screen_is_replaced_without_losing_input() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    for (description, preset, days) in [
        (
            "Bad Request: message to edit not found",
            texts::PRESET_WEEK,
            7,
        ),
        (
            "Bad Request: message can't be edited",
            texts::PRESET_TWO_WEEKS,
            14,
        ),
    ] {
        let old = open_terms(&app, &telegram).await;
        let first = preset_deadline(days);
        telegram.click_preset(app.teacher(), preset).await;
        let old_save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
        telegram
            .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
            .await;
        reject_message_edits(&telegram, old.message_id, description).await;
        telegram.message(app.teacher(), "/cancel");
        telegram.flush(app.teacher()).await;
        let new_save = telegram
            .button_after(app.teacher(), callbacks::TERMS_SAVE, old.message_id)
            .await;
        let before = telegram.messages(app.teacher()).len();
        telegram.click(app.teacher(), &old_save);
        telegram.flush(app.teacher()).await;
        assert!(
            telegram.messages(app.teacher())[before..]
                .iter()
                .any(|m| m["text"] == texts::STALE_BUTTON)
        );
        telegram.click(app.teacher(), &new_save);
        telegram.flush(app.teacher()).await;
        let assignment = app
            .application
            .assignments
            .find_for_teacher(app.teacher_actor(), app.fixture.assignment.id())
            .await
            .unwrap();
        assert!(
            [first, preset_deadline(days)].contains(&assignment.deadline_at()),
            "{days} days"
        );
    }
    stop_bot(cancellation, &mut task).await;
}

fn preset_deadline(days: u64) -> DateTime<Utc> {
    let timezone = vladivostok();
    timezone
        .end_of(timezone.in_days(Utc::now(), days).unwrap())
        .unwrap()
}

#[tokio::test]
async fn deleted_terms_screen_does_not_block_exit_commands() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    for command in ["/start", "/menu", "/courses"] {
        let old = open_terms(&app, &telegram).await;
        reject_message_edits(
            &telegram,
            old.message_id,
            "Bad Request: message to edit not found",
        )
        .await;
        let before = telegram.messages(app.teacher()).len();
        telegram.message(app.teacher(), command);
        telegram.message(app.teacher(), "input after leaving");
        telegram.flush(app.teacher()).await;
        let messages = telegram.messages(app.teacher());
        assert!(
            messages[before..]
                .iter()
                .any(|m| m["text"] == texts::TERMS_DISCARDED),
            "{command}"
        );
        assert!(
            messages[before..]
                .iter()
                .any(|m| m["text"] == texts::USE_THE_MENU),
            "{command}"
        );
    }
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn replayable_navigation_leaves_deleted_terms_screen() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = open_terms(&app, &telegram).await;
    let course = telegram
        .button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    reject_message_edits(
        &telegram,
        old.message_id,
        "Bad Request: message to edit not found",
    )
    .await;
    let before = telegram.messages(app.teacher()).len();
    telegram.click(app.teacher(), &course);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let messages = telegram.messages(app.teacher());
    assert!(
        messages[before..]
            .iter()
            .any(|m| m["text"] == texts::TERMS_DISCARDED)
    );
    assert!(
        messages[before..]
            .iter()
            .any(|m| m["text"] == texts::CHOOSE_ASSIGNMENT)
    );
}

#[tokio::test]
async fn already_removed_markup_does_not_block_manual_deadline_input() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = open_terms(&app, &telegram).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/EditMessageReplyMarkup"))
        .and(body_partial_json(json!({"message_id": old.message_id})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400,
            "description": teloxide::ApiError::MessageNotModified.to_string(),
        })))
        .expect(1)
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    let date = deadline_input(Utc::now() + chrono::Duration::days(5));
    telegram.message(app.teacher(), &date);
    telegram.flush(app.teacher()).await;
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    telegram.click(app.teacher(), &save);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let assignment = app
        .application
        .assignments
        .find_for_teacher(app.teacher_actor(), app.fixture.assignment.id())
        .await
        .unwrap();
    assert_eq!(assignment.deadline_at(), expected_deadline(&date));
}

#[tokio::test]
async fn failed_keyboard_cleanup_still_closes_the_form() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_terms(&app, &telegram).await;
    fail_next(&telegram, "EditMessageReplyMarkup").await;
    telegram.message(app.teacher(), "/courses");
    telegram.message(app.teacher(), "input after leaving");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|m| m["text"] == texts::USE_THE_MENU)
    );
}

async fn add_assignment_document(app: &TestApp) {
    let revision = app
        .application
        .assignments
        .find_versioned_for_teacher(app.teacher_actor(), app.fixture.assignment.id())
        .await
        .unwrap()
        .revision;
    app.application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            expected_revision: revision,
            presentation: FilePresentation::Document,
            filename: "assignment.txt".into(),
            mime_type: Some("text/plain".into()),
            telegram_file_id: None,
            content: DOCUMENT_FIXTURE.to_vec(),
            created_at: Utc::now(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn failed_attachment_delivery_keeps_actions_and_can_be_retried() {
    let app = TestApp::start().await;
    add_assignment_document(&app).await;
    let previous = app.queued_submission(app.student(), "pull/first").await;
    app.review(previous, app.teacher(), "reject", "Try again")
        .await
        .ok();
    let telegram = TelegramScenario::new().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false, "error_code": 429,
            "description": "Too Many Requests: retry after 5",
            "parameters": {"retry_after": 5},
        })))
        .expect(1..)
        .with_priority(10)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_assignment(&app, &telegram, app.assignment()).await;
    assert!(offers_submission(&telegram, app.student()));
    assert!(
        telegram
            .messages(app.student())
            .iter()
            .any(|m| m["text"] == texts::ASSIGNMENT_FILES_SEND_FAILED)
    );
    let history = telegram
        .button_with_text(app.student(), texts::BUTTON_HISTORY)
        .await;
    let retry = telegram
        .button_with_text(app.student(), texts::BUTTON_REOPEN_ASSIGNMENT)
        .await;
    let submit = telegram
        .button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    let before = telegram.messages(app.student()).len();
    telegram.click(app.student(), &history);
    telegram.flush(app.student()).await;
    let messages = telegram.messages(app.student());
    let history_text = messages[before..]
        .iter()
        .filter_map(|message| message["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(history_text.contains("pull/first"));
    assert!(history_text.contains("Try again"));
    telegram.click(app.student(), &submit);
    telegram.message(app.student(), "pull/retry");
    telegram.flush(app.student()).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response())
        .expect(1..)
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    telegram.click(app.student(), &retry);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn attachment_timeout_keeps_submission_available() {
    let app = TestApp::start().await;
    add_assignment_document(&app).await;
    let telegram = TelegramScenario::new().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(telegram_document_response().set_delay(Duration::from_secs(1)))
        .expect(1..)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot_with_settings(
        &app,
        &telegram.server,
        bot_settings_with(|settings| {
            settings.file_transfer_timeout = Duration::from_millis(250);
        }),
    );
    open_assignment(&app, &telegram, app.assignment()).await;
    assert!(offers_submission(&telegram, app.student()));
    assert!(
        telegram
            .messages(app.student())
            .iter()
            .any(|m| m["text"] == texts::ASSIGNMENT_FILES_SEND_FAILED)
    );
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await;
    telegram.message(app.student(), "pull/after-timeout");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

struct RejectedDelivery {
    cancellation: CancellationToken,
    result: Result<(), TelegramBotError>,
    telegram: MockServer,
}

async fn deliver_an_attachment_with_rejected_credentials(app: &TestApp) -> RejectedDelivery {
    add_assignment_document(app).await;
    let telegram = telegram_server(vec![callback_update(
        1,
        app.student(),
        &cb(callbacks::ASSIGNMENT_PREFIX, app.assignment()),
        BOT_MESSAGE_ID,
    )])
    .await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendDocument"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false, "error_code": 401, "description": "Unauthorized",
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, task) = start_bot(app, &telegram);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    RejectedDelivery {
        cancellation,
        result,
        telegram,
    }
}

#[tokio::test]
async fn invalid_credentials_stop_the_bot_without_cancelling_the_other_components() {
    let app = TestApp::start().await;

    let delivery = deliver_an_attachment_with_rejected_credentials(&app).await;

    assert!(matches!(
        delivery.result,
        Err(TelegramBotError::InvalidCredentials)
    ));
    assert!(!delivery.cancellation.is_cancelled());
}

#[tokio::test]
async fn invalid_credentials_during_attachment_delivery_are_not_reported_as_a_failed_delivery() {
    let app = TestApp::start().await;

    let delivery = deliver_an_attachment_with_rejected_credentials(&app).await;

    assert!(matches!(
        delivery.result,
        Err(TelegramBotError::InvalidCredentials)
    ));
    assert!(
        !recorded_requests(&delivery.telegram)
            .await
            .contains(texts::ASSIGNMENT_FILES_SEND_FAILED)
    );
}

#[tokio::test]
async fn failed_confirmation_does_not_create_a_second_assignment() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_course(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), "Unique draft");
    telegram.message(app.teacher(), "Solve it");
    telegram.message(app.teacher(), "2");
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADING_PREFIX, "passfail"))
        .await;
    telegram
        .click_preset(app.teacher(), texts::PRESET_WEEK)
        .await;
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    fail_next(&telegram, "SendMessage").await;
    telegram.click(app.teacher(), &save);
    wait_for_created_assignment(&app.pool, "Unique draft")
        .await
        .expect("committed draft");
    telegram.click(app.teacher(), &save);
    telegram.message(
        app.teacher(),
        &deadline_input(Utc::now() + chrono::Duration::days(7)),
    );
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let count: i64 =
        sqlx::query_scalar("select count(*) from assignments where title = 'Unique draft'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn failed_confirmation_does_not_grant_attempts_twice() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    manage_assignment(&app, &telegram, app.assignment()).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::ATTEMPTS_PREFIX, app.assignment()),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(
                callbacks::STUDENT_PREFIX,
                app.fixture.student.id().as_uuid(),
            ),
        )
        .await;
    let amount = telegram
        .button(app.teacher(), &cb(callbacks::EXTRA_ATTEMPTS_PREFIX, 1))
        .await;
    fail_next(&telegram, "SendMessage").await;
    telegram.click(app.teacher(), &amount);
    assert!(
        wait_for_extra_attempts(
            &app.pool,
            app.assignment(),
            app.fixture.student.id().as_uuid(),
            1
        )
        .await
    );
    telegram.click(app.teacher(), &amount);
    telegram.message(app.teacher(), "1");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert!(
        wait_for_extra_attempts(
            &app.pool,
            app.assignment(),
            app.fixture.student.id().as_uuid(),
            1
        )
        .await
    );
}

#[tokio::test]
async fn an_acknowledgement_failure_does_not_discard_a_valid_action() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    manage_assignment(&app, &telegram, app.assignment()).await;
    let archive = telegram
        .button(
            app.teacher(),
            &cb(callbacks::ARCHIVE_PREFIX, app.assignment()),
        )
        .await;
    fail_next(&telegram, "AnswerCallbackQuery").await;
    telegram.click(app.teacher(), &archive);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(app.assignment())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "archived");
}

#[tokio::test]
async fn reopening_a_screen_invalidates_the_actual_previous_message() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let manage = manage_assignment(&app, &telegram, app.assignment()).await;
    let old = telegram
        .button(
            app.teacher(),
            &cb(callbacks::ARCHIVE_PREFIX, app.assignment()),
        )
        .await;
    telegram.click(app.teacher(), &manage);
    let new = telegram
        .button_after(
            app.teacher(),
            &cb(callbacks::ARCHIVE_PREFIX, app.assignment()),
            old.message_id,
        )
        .await;
    assert_ne!(new.message_id, old.message_id);
    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(app.assignment())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "published");
}

#[tokio::test]
async fn forced_shutdown_stops_active_handlers_and_queued_mutations() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let cancellation = CancellationToken::new();
    let health = HealthState::new(true, false);
    let bot = TelegramBot::new(
        app.application.clone(),
        "test-token",
        &telegram.server.uri(),
        bot_settings_with(|settings| settings.request_timeout = Duration::from_secs(30)),
        vladivostok(),
        Clock::system(),
        crate::harness::invite_codec(),
    )
    .unwrap();
    let mut supervisor = Supervisor::new(
        cancellation.clone(),
        health.clone(),
        Duration::from_millis(100),
    );
    let bot_cancellation = cancellation.clone();
    supervisor.spawn(CriticalTask::TelegramBot, async move {
        bot.run(
            bot_cancellation,
            health.component(CriticalComponent::TelegramBot),
        )
        .await
        .map_err(Into::into)
    });
    let signal = cancellation.clone();
    let task = tokio::spawn(supervisor.run_with_cleanup(
        async move {
            signal.cancelled().await;
            Ok(())
        },
        async { Ok(()) },
    ));
    open_submission_prompt(&app, &telegram).await;
    telegram.flush(app.student()).await;
    let before = sent_message_count(&telegram.server).await;
    let gate = gate_sent_messages(&telegram.server, 1).await;
    telegram.message(app.student(), "/help");
    telegram.message(app.student(), "pull/should-not-be-written");
    gate.wait_started().await;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    gate.wait_finished().await;
    assert_eq!(sent_message_count(&telegram.server).await, before + 1);
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
