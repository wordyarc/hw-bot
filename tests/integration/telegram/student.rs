use std::time::Duration;

use chrono::{DateTime, Utc};
use hw_bot::bot::callbacks;
use hw_bot::db;
use hw_bot::domain::{Assignment, Submission};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;

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
async fn a_delayed_submission_does_not_get_a_penalty_for_processing_time() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(0, Some((1, 1, 4))).await;
    let sent_at = DateTime::from_timestamp(assignment.deadline_at().timestamp() - 1, 0).unwrap();

    let submission = delayed_submission(&app, &assignment, sent_at).await;
    app.review_with_score(
        submission.id().as_uuid(),
        app.teacher(),
        "accept",
        5,
        "Принято",
    )
    .await
    .ok();
    let reviewed = db::submissions::find_by_id(&app.pool, submission.id())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(reviewed.grade().unwrap().penalty(), 0);
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
