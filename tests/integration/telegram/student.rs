use std::time::Duration;

use hw_bot::bot::{callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;

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
async fn a_student_sees_what_a_late_submission_costs_right_now() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!(
                "{}{}",
                callbacks::ASSIGNMENT_PREFIX,
                assignment.id().as_uuid()
            ),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 5).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert!(
        requests.contains(&format!("{} −1", texts::LATE_PENALTY_LABEL)),
        "requests:\n{requests}"
    );
    assert!(
        requests.contains(texts::SUBMISSIONS_CLOSE_LABEL),
        "requests:\n{requests}"
    );
}
