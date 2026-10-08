use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hw_bot::app::ErrorCode;
use hw_bot::bot::{TelegramBotError, app_error_message, callbacks, texts};
use hw_bot::domain::{SubmissionId, TeachingCourseId, UserId};
use hw_bot::test_support::telegram::private_chat_info;
use serde_json::{Value, json};
use wiremock::matchers::{body_json, body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{Button, TelegramScenario};
use crate::harness::{TestApp, submission_id};

async fn mount_contact(
    telegram: &TelegramScenario,
    student: i64,
    username: Option<&str>,
    restricted: bool,
) {
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetChat"))
        .and(body_json(json!({"chat_id": student})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "result": private_chat_info(student, username, restricted)
        })))
        .with_priority(10)
        .mount(&telegram.server)
        .await;
}

async fn refresh_after_username_change(updated: Option<&str>) -> Value {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("old_username"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    let original = message(&telegram, app.teacher(), button.message_id);
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let old = message(&telegram, app.teacher(), refresh.message_id);
    assert_eq!(urls(&old), ["https://t.me/old_username"]);

    Mock::given(path("/bottest-token/GetChat"))
        .and(body_json(json!({"chat_id": app.student()})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "result": private_chat_info(app.student(), updated, true)
        })))
        .with_priority(1)
        .expect(1)
        .mount(&telegram.server)
        .await;
    telegram.click(app.teacher(), &refresh);
    telegram.flush(app.teacher()).await;
    let refreshed = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refreshed.message_id);
    stop_bot(cancellation, &mut task).await;

    assert!(refreshed.message_id > refresh.message_id);
    assert_eq!(requests(&telegram, "GetChat").await.len(), 2);
    assert_eq!(
        message(&telegram, app.teacher(), button.message_id),
        original
    );
    assert_eq!(message(&telegram, app.teacher(), refresh.message_id), old);
    card
}

#[tokio::test]
async fn refresh_uses_the_current_username_and_preserves_gradebook_navigation() {
    let card = refresh_after_username_change(Some("new_username")).await;
    assert_eq!(urls(&card), ["https://t.me/new_username"]);
    assert!(card["text"].as_str().unwrap().contains("@new_username"));
    assert!(!card["text"].as_str().unwrap().contains("old_username"));
}

#[tokio::test]
async fn refresh_never_falls_back_to_a_removed_username() {
    let card = refresh_after_username_change(None).await;
    assert!(urls(&card).is_empty());
    assert!(
        card["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_RESTRICTED)
    );
    assert!(!card["text"].as_str().unwrap().contains("old_username"));
}

async fn requests(telegram: &TelegramScenario, endpoint: &str) -> Vec<Value> {
    telegram
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path() == format!("/bottest-token/{endpoint}"))
        .map(|request| request.body_json().unwrap())
        .collect()
}

fn message(telegram: &TelegramScenario, user: i64, id: i32) -> Value {
    telegram
        .messages(user)
        .into_iter()
        .find(|message| message["message_id"] == id)
        .unwrap()
}

fn urls(message: &Value) -> Vec<&str> {
    message["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap())
        .filter_map(|button| button["url"].as_str())
        .collect()
}

async fn review_contact(app: &TestApp, telegram: &TelegramScenario) -> Button {
    telegram.message(app.teacher(), "/review");
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_STUDENT_CONTACT)
        .await
}

async fn review_comment(
    app: &TestApp,
    telegram: &TelegramScenario,
    submission: uuid::Uuid,
    score: Option<u16>,
) -> Button {
    let contact = review_contact(app, telegram).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    if let Some(score) = score {
        telegram
            .click_button(
                app.teacher(),
                &cb(
                    callbacks::REVIEW_SCORE_PREFIX,
                    format!("{submission}:{score}"),
                ),
            )
            .await;
    }
    contact
}

async fn assert_saved_review(
    app: &TestApp,
    submission: uuid::Uuid,
    score: Option<u16>,
    comment: &str,
) {
    let saved = hw_bot::db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.status(), hw_bot::domain::SubmissionStatus::Accepted);
    assert_eq!(saved.review_comment().unwrap().as_str(), comment);
    assert_eq!(saved.grade().map(|grade| grade.raw_score()), score);
}

#[tokio::test]
async fn lookup_timeout_can_be_retried_without_losing_the_review_score() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/timeout")
            .await
            .created(),
    );
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("recovered_name"), false).await;
    Mock::given(path("/bottest-token/GetChat"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)).set_body_json(json!({
            "ok": true, "result": private_chat_info(app.student(), Some("late_response"), false)
        })))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&telegram.server).await;
    let (cancellation, mut task) = start_bot_with_settings(
        &app,
        &telegram.server,
        bot_settings_with(|s| {
            s.request_timeout = Duration::from_millis(300);
        }),
    );
    let button = review_comment(&app, &telegram, submission, Some(4)).await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let retry = telegram
        .button_with_text(app.teacher(), texts::BUTTON_RETRY_CONTACT)
        .await;
    let failed = message(&telegram, app.teacher(), retry.message_id);
    assert!(
        failed["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_LOOKUP_FAILED)
    );
    assert!(urls(&failed).is_empty());
    telegram.click(app.teacher(), &retry);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    assert_eq!(
        urls(&message(&telegram, app.teacher(), refresh.message_id)),
        ["https://t.me/recovered_name"]
    );
    assert_eq!(requests(&telegram, "GetChat").await.len(), 2);
    telegram.message(app.teacher(), "Still reviewing the same submission");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(
        &app,
        submission,
        Some(4),
        "Still reviewing the same submission",
    )
    .await;
}

#[tokio::test]
async fn unavailable_account_is_explained_without_claiming_privacy() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    Mock::given(path("/bottest-token/GetChat"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400, "description": "Bad Request: chat not found"
        })))
        .expect(1)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    stop_bot(cancellation, &mut task).await;
    assert!(
        card["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_UNAVAILABLE)
    );
    assert!(
        !card["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_RESTRICTED)
    );
    assert!(urls(&card).is_empty());
}

#[tokio::test]
async fn private_forwards_without_username_shows_explanation_and_refresh() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), None, true).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    stop_bot(cancellation, &mut task).await;
    assert!(
        card["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_RESTRICTED)
    );
    assert!(urls(&card).is_empty());
}

#[tokio::test]
async fn rejected_id_link_falls_back_once_and_keeps_the_review_comment_step() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/privacy").await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), None, false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = review_comment(&app, &telegram, submission, None).await;
    Mock::given(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({"reply_parameters": {"message_id": button.message_id}})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400, "description": "Bad Request: BUTTON_USER_PRIVACY_RESTRICTED"
        })))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&telegram.server).await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    assert!(
        card["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_RESTRICTED)
    );
    assert!(urls(&card).is_empty());
    let attempts: Vec<_> = requests(&telegram, "SendMessage")
        .await
        .into_iter()
        .filter(|body| body["reply_parameters"]["message_id"] == button.message_id)
        .collect();
    assert_eq!(attempts.len(), 2);
    assert_eq!(
        urls(&attempts[0]),
        [format!("tg://user?id={}", app.student())]
    );
    assert!(urls(&attempts[1]).is_empty());
    telegram.message(app.teacher(), "After privacy rejection");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, submission, None, "After privacy rejection").await;
}

#[tokio::test]
async fn uncertain_card_send_is_not_retried_and_the_review_continues() {
    let app = TestApp::start().await;
    let submission = app
        .queued_submission(app.student(), "pull/send-timeout")
        .await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), None, false).await;
    let (cancellation, mut task) = start_bot_with_settings(
        &app,
        &telegram.server,
        bot_settings_with(|s| {
            s.request_timeout = Duration::from_millis(300);
        }),
    );
    let button = review_comment(&app, &telegram, submission, None).await;
    Mock::given(path("/bottest-token/SendMessage"))
        .and(body_partial_json(
            json!({"reply_parameters": {"message_id": button.message_id}}),
        ))
        .respond_with(telegram_message_response().set_delay(Duration::from_secs(2)))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&telegram.server)
        .await;
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    assert_eq!(
        requests(&telegram, "SendMessage")
            .await
            .iter()
            .filter(|body| body["reply_parameters"]["message_id"] == button.message_id)
            .count(),
        1
    );
    assert!(!telegram.messages(app.teacher()).iter().any(|m| {
        m["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_RESTRICTED)
    }));
    telegram.message(app.teacher(), "After uncertain delivery");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, submission, None, "After uncertain delivery").await;
}

#[tokio::test]
async fn invalid_credentials_during_contact_lookup_stop_the_bot() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    Mock::given(path("/bottest-token/GetChat"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false, "error_code": 401, "description": "Unauthorized"
        })))
        .expect(1)
        .mount(&telegram.server)
        .await;
    let (_, task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    telegram.click(app.teacher(), &button);
    let result = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(TelegramBotError::InvalidCredentials)));
    assert!(!telegram.messages(app.teacher()).iter().any(|m| {
        m["text"]
            .as_str()
            .unwrap()
            .contains(texts::CONTACT_LOOKUP_FAILED)
    }));
}

#[tokio::test]
async fn contact_preserves_the_selected_score_and_comment_recipient() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            app.student(),
            "pull/scored-contact",
        )
        .await
        .created(),
    );
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_name"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let contact = review_comment(&app, &telegram, submission, Some(4)).await;
    telegram.click(app.teacher(), &contact);
    telegram.flush(app.teacher()).await;
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    telegram.message(app.teacher(), "Original decision and score");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, submission, Some(4), "Original decision and score").await;
}

#[tokio::test]
async fn contact_preserves_review_b_when_opened_from_old_student_a_button() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let other = app.add_student("Second student").await;
    let first = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/a")
            .await
            .created(),
    );
    let second = submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            other.telegram_user_id().as_i64(),
            "pull/b",
        )
        .await
        .created(),
    );
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_a"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = review_comment(&app, &telegram, first, Some(3)).await;
    telegram.message(app.teacher(), "Review A");
    telegram.flush(app.teacher()).await;
    review_comment(&app, &telegram, second, Some(4)).await;

    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    assert!(
        card["text"]
            .as_str()
            .unwrap()
            .contains(app.fixture.student.display_name().as_str())
    );
    assert_eq!(urls(&card), ["https://t.me/student_a"]);
    assert_eq!(
        requests(&telegram, "GetChat").await,
        [json!({"chat_id": app.student()})]
    );
    telegram.message(app.teacher(), "Review B");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, first, Some(3), "Review A").await;
    assert_saved_review(&app, second, Some(4), "Review B").await;
}

#[tokio::test]
async fn contact_preserves_entered_terms_and_their_active_keyboard() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/old-contact")
        .await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_name"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let contact = review_contact(&app, &telegram).await;
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), "Contact-safe draft");
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
    let original = message(&telegram, app.teacher(), periods.message_id);

    telegram.click(app.teacher(), &contact);
    telegram.flush(app.teacher()).await;
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    assert_eq!(
        message(&telegram, app.teacher(), periods.message_id),
        original
    );
    assert!(!telegram.messages(app.teacher()).iter().any(|message| {
        let text = message["text"].as_str().unwrap();
        text.contains(texts::DRAFT_DISCARDED) || text.contains(texts::TERMS_DISCARDED)
    }));
    telegram.click(app.teacher(), &periods);
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::FILE_FINISH)
        .await;
    stop_bot(cancellation, &mut task).await;

    let created = wait_for_created_assignment(&app.pool, "Contact-safe draft")
        .await
        .unwrap();
    assert_eq!(created.body, "Keep this body");
    assert_eq!(created.max_attempts, 3);
    assert_eq!(created.max_score, Some(5));
    assert_eq!(created.deadline_at, expected_deadline(&deadline));
    let saved = hw_bot::db::assignments::find_by_id(
        &app.pool,
        hw_bot::domain::AssignmentId::from_uuid(created.id),
    )
    .await
    .unwrap()
    .unwrap();
    let policy = saved.late_policy().unwrap();
    assert_eq!(
        (
            policy.period_days(),
            policy.penalty_per_period(),
            policy.max_periods()
        ),
        (7, 2, 4)
    );
}

#[tokio::test]
async fn old_contact_works_after_restart_without_restoring_the_comment_step() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/restart").await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_name"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = review_comment(&app, &telegram, submission, None).await;
    stop_bot(cancellation, &mut task).await;

    Mock::given(path("/bottest-token/GetWebhookInfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true,
            "result": {"url": "", "has_custom_certificate": false, "pending_update_count": 0}
        })))
        .with_priority(1)
        .expect(1)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    assert_eq!(
        urls(&message(&telegram, app.teacher(), refresh.message_id)),
        ["https://t.me/student_name"]
    );
    telegram.message(app.teacher(), "This must not complete the old review");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let saved = hw_bot::db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.status(),
        hw_bot::domain::SubmissionStatus::PendingReview
    );
    assert!(saved.review_comment().is_none());
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|m| m["text"] == texts::USE_THE_MENU)
    );
    assert_eq!(requests(&telegram, "GetChat").await.len(), 1);
}

#[tokio::test]
async fn revoked_access_is_rechecked_before_telegram_and_keeps_the_active_step() {
    let app = TestApp::start().await;
    app.add_course_teacher("Remaining teacher").await;
    let submission = app.queued_submission(app.student(), "pull/revoked").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = review_comment(&app, &telegram, submission, None).await;
    app.remove_course_teacher(&app.fixture.teacher).await;
    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    assert!(requests(&telegram, "GetChat").await.is_empty());
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|m| m["text"] == app_error_message(ErrorCode::PermissionDenied))
    );

    sqlx::query(
        "insert into teaching_course_teachers (teaching_course_id, teacher_id) values ($1, $2)",
    )
    .bind(app.course())
    .bind(app.fixture.teacher.id().as_uuid())
    .execute(&app.pool)
    .await
    .unwrap();
    telegram.message(app.teacher(), "Continue after restoring access");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, submission, None, "Continue after restoring access").await;
}

#[tokio::test]
async fn tampered_student_is_rejected_before_telegram_without_resetting_review() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_student().await;
    let submission = app.queued_submission(app.student(), "pull/tampered").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let old = review_comment(&app, &telegram, submission, None).await;
    let forged = old.with_data(format!(
        "{}{}:{}",
        callbacks::CONTACT_PREFIX,
        URL_SAFE_NO_PAD.encode(app.course().as_bytes()),
        URL_SAFE_NO_PAD.encode(outsider.id().as_uuid().as_bytes()),
    ));
    telegram.click(app.teacher(), &forged);
    telegram.flush(app.teacher()).await;
    assert!(requests(&telegram, "GetChat").await.is_empty());
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|m| m["text"] == app_error_message(ErrorCode::NotFound))
    );
    telegram.message(app.teacher(), "Continue for the real student");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_saved_review(&app, submission, None, "Continue for the real student").await;
}

#[tokio::test]
async fn guest_clicking_a_copied_contact_button_gets_the_access_explanation() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    telegram.click(guest, &button.in_chat(guest));
    assert!(
        wait_until(Duration::from_secs(5), || async {
            telegram
                .messages(guest)
                .iter()
                .any(|m| m["text"] == texts::GUEST_NEEDS_INVITE)
        })
        .await
    );
    stop_bot(cancellation, &mut task).await;
    assert!(requests(&telegram, "GetChat").await.is_empty());
    assert!(telegram.messages(guest).iter().any(|m| {
        m["text"] == texts::GUEST_NEEDS_INVITE && m["reply_markup"]["remove_keyboard"] == true
    }));
}

#[tokio::test]
async fn contact_is_available_on_each_grade_page_and_keeps_pagination_working() {
    let app = TestApp::start().await;
    for _ in 0..=hw_bot::pagination::PAGE_SIZE {
        app.add_scored_assignment(5).await;
    }
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_name"), false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let first = gradebook_contact(
        &app,
        &telegram,
        app.fixture.course.id(),
        app.fixture.student.id(),
    )
    .await;
    let original = message(&telegram, app.teacher(), first.message_id);
    telegram.click(app.teacher(), &first);
    telegram.flush(app.teacher()).await;
    let next = telegram
        .button_with_text(app.teacher(), texts::BUTTON_NEXT_PAGE)
        .await;
    telegram.click(app.teacher(), &next);
    telegram.flush(app.teacher()).await;
    let second = telegram
        .button_with_text(app.teacher(), texts::BUTTON_STUDENT_CONTACT)
        .await;
    assert!(second.message_id > first.message_id);
    let page = message(&telegram, app.teacher(), second.message_id);
    assert_eq!(
        original["reply_markup"]["inline_keyboard"][0][0],
        page["reply_markup"]["inline_keyboard"][0][0]
    );
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_PREVIOUS_PAGE)
        .await;
    telegram.click(app.teacher(), &second);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(message(&telegram, app.teacher(), second.message_id), page);
    assert_eq!(requests(&telegram, "GetChat").await.len(), 2);
}

async fn gradebook_contact(
    app: &TestApp,
    telegram: &TelegramScenario,
    course: TeachingCourseId,
    student: UserId,
) -> Button {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::COURSE_PREFIX, course.as_uuid()),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::GRADES_PREFIX, course.as_uuid()),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::STUDENT_PREFIX, student.as_uuid()),
        )
        .await;
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_STUDENT_CONTACT)
        .await
}

#[tokio::test]
async fn review_opens_registered_contact_only_on_click_as_an_optional_reply() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/contact").await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), Some("student_name"), true).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    let button = review_contact(&app, &telegram).await;
    let original = message(&telegram, app.teacher(), button.message_id);
    assert!(requests(&telegram, "GetChat").await.is_empty());
    assert_eq!(
        original["reply_markup"]["inline_keyboard"][0]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        original["reply_markup"]["inline_keyboard"][1][0]["text"],
        texts::BUTTON_STUDENT_CONTACT
    );
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        requests(&telegram, "GetChat").await,
        vec![json!({"chat_id": app.student()})]
    );
    let text = card["text"].as_str().unwrap();
    assert!(text.contains(app.fixture.student.display_name().as_str()));
    assert!(text.contains(app.fixture.group.code().as_str()));
    assert!(!text.contains("Telegram account name"));
    assert!(text.contains(texts::CONTACT_PERSONAL_ACCOUNT));
    assert_eq!(urls(&card), ["https://t.me/student_name"]);
    assert_eq!(
        message(&telegram, app.teacher(), button.message_id),
        original
    );
    assert!(telegram.messages(app.student()).is_empty());
    let sent = requests(&telegram, "SendMessage").await;
    let card_request = sent
        .iter()
        .find(|body| body["text"] == card["text"])
        .unwrap();
    assert_eq!(
        card_request["reply_parameters"],
        json!({
            "message_id": button.message_id, "allow_sending_without_reply": true
        })
    );
    assert!(card_request["parse_mode"].is_null());
    assert!(card_request["reply_markup"]["remove_keyboard"].is_null());
    assert!(card_request["reply_markup"]["keyboard"].is_null());
}

#[tokio::test]
async fn gradebook_offers_contact_without_submissions_or_graded_assignments() {
    let app = TestApp::start().await;
    let course = app.add_subject_course("Empty course").await;
    let telegram = TelegramScenario::new().await;
    mount_contact(&telegram, app.student(), None, false).await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let button = gradebook_contact(&app, &telegram, course.id(), app.fixture.student.id()).await;
    let original = message(&telegram, app.teacher(), button.message_id);
    assert!(
        original["text"]
            .as_str()
            .unwrap()
            .contains(texts::NO_SCORED_ASSIGNMENTS)
    );
    assert!(requests(&telegram, "GetChat").await.is_empty());
    telegram.click(app.teacher(), &button);
    telegram.flush(app.teacher()).await;
    let refresh = telegram
        .button_with_text(app.teacher(), texts::BUTTON_REFRESH_CONTACT)
        .await;
    let card = message(&telegram, app.teacher(), refresh.message_id);
    assert_eq!(urls(&card), [format!("tg://user?id={}", app.student())]);
    assert_eq!(
        message(&telegram, app.teacher(), button.message_id),
        original
    );

    let back = telegram
        .button_with_text(app.teacher(), texts::BUTTON_BACK_TO_STUDENTS)
        .await;
    telegram.click(app.teacher(), &back);
    telegram.flush(app.teacher()).await;
    telegram
        .button_after(
            app.teacher(),
            &cb(
                callbacks::STUDENT_PREFIX,
                app.fixture.student.id().as_uuid(),
            ),
            back.message_id,
        )
        .await;
    stop_bot(cancellation, &mut task).await;
}
