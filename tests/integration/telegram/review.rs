use std::time::Duration;

use hw_bot::bot::{callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, method, path};

use crate::harness::telegram::*;
use crate::harness::{TestApp, submission_id};

#[tokio::test]
async fn reviewer_enters_a_score_before_the_comment() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::NEXT_REVIEW, BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("{}{submission}", callbacks::REVIEW_ACCEPT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            4,
            app.teacher(),
            &cb(callbacks::REVIEW_SCORE_PREFIX, 4),
            BOT_MESSAGE_ID,
        ),
        message_update(5, app.teacher(), 2, "Хорошо"),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let reviewed = wait_for_review_result(&app.pool, submission).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let (status, raw_score, final_score) =
        reviewed.unwrap_or_else(|| panic!("submission was not reviewed; requests:\n{requests}"));
    assert_eq!(status, "accepted");
    assert_eq!(raw_score, Some(4));
    assert_eq!(final_score, Some(4));
}

#[tokio::test]
async fn a_pass_fail_review_never_asks_for_a_score() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::NEXT_REVIEW, BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("{}{submission}", callbacks::REVIEW_ACCEPT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        message_update(4, app.teacher(), 2, "Зачтено"),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let reviewed = wait_for_review_result(&app.pool, submission).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let (status, raw_score, _) =
        reviewed.unwrap_or_else(|| panic!("submission was not reviewed; requests:\n{requests}"));
    assert_eq!(status, "accepted");
    assert_eq!(
        raw_score, None,
        "the comment followed the decision directly"
    );
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
            "text": texts::REVIEW_QUEUE_EMPTY,
            "reply_markup": {
                "inline_keyboard": [
                    [{ "text": "✅ Проверить работы (0)", "callback_data": callbacks::NEXT_REVIEW }],
                    [{ "text": "📘 Мои курсы", "callback_data": callbacks::MENU_COURSES }],
                    [{ "text": "👤 Профиль", "callback_data": callbacks::MENU_PROFILE }]
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
async fn teacher_opens_the_gradebook_and_drills_into_a_student() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    app.review_with_score(submission, app.teacher(), "accept", 4, "Ок")
        .await
        .ok();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("{}{}", callbacks::GRADES_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            3,
            app.teacher(),
            &format!(
                "{}{}",
                callbacks::STUDENT_PREFIX,
                app.fixture.student.id().as_uuid()
            ),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 4).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert!(
        requests.contains("👤 Student · 4 из 5"),
        "the list carries the total; requests:\n{requests}"
    );
    assert!(
        requests.contains("Rust traits · 4 из 5"),
        "the drill-down carries the per-assignment score; requests:\n{requests}"
    );
}
