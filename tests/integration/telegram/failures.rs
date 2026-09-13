use std::time::Duration;

use chrono::Utc;
use hw_bot::bot::{BotTimezone, TelegramBot, callbacks};
use hw_bot::health::{CriticalComponent, HealthState};
use hw_bot::supervision::{CriticalTask, Supervisor};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;
use hw_bot::app::ErrorCode;
use hw_bot::bot::app_error_message;

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

#[tokio::test]
async fn a_submission_error_preserves_the_input_step_for_retry() {
    let app = TestApp::start().await;
    let pending = app.queued_submission(app.student(), "pull/first").await;
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
    telegram.message(app.student(), "pull/retry");
    telegram.flush(app.student()).await;
    assert_eq!(
        telegram
            .messages(app.student())
            .iter()
            .filter(
                |message| message["text"] == app_error_message(ErrorCode::SubmissionPendingReview)
            )
            .count(),
        1
    );

    app.review(pending, app.teacher(), "reject", "Retry")
        .await
        .ok();
    telegram.message(app.student(), "pull/retry");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

async fn open_course(app: &TestApp, telegram: &TelegramScenario) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
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
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
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
    open_course(&app, &telegram).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::MANAGE_PREFIX, app.assignment()),
        )
        .await;
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
    open_course(&app, &telegram).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::MANAGE_PREFIX, app.assignment()),
        )
        .await;
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
    open_course(&app, &telegram).await;
    let manage = telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::MANAGE_PREFIX, app.assignment()),
        )
        .await;
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
        bot_settings_with(|_| {}),
        BotTimezone::new(
            chrono::FixedOffset::east_opt(36000).unwrap(),
            chrono::NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        ),
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
    telegram.flush(app.student()).await;
    let before = sent_message_count(&telegram.server).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(telegram_message_response().set_delay(Duration::from_millis(600)))
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    telegram.message(app.student(), "/help");
    telegram.message(app.student(), "pull/should-not-be-written");
    wait_for_sent_message_count(&telegram.server, before + 1).await;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(sent_message_count(&telegram.server).await, before + 1);
    let count: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
