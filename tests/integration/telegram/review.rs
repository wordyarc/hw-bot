use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use hw_bot::app::ErrorCode;
use hw_bot::bot::{app_error_message, callbacks, texts};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, DeductedFrom, Grade, ReviewDecision, SubmissionId, SubmissionKind,
    SubmissionStatus, USER_TEXT_MAX_LEN,
};
use hw_bot::test_support::{assignment as assignment_of, history, submission as submission_of};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, ResponseTemplate};

use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;
use crate::harness::{TestApp, out_of, submission_id};

fn regular_grade(grade: Grade) -> String {
    texts::grade_line(SubmissionKind::Regular, Some(grade)).unwrap()
}

#[tokio::test]
async fn the_review_card_shows_ten_previous_attempts_without_a_notice() {
    assert_previous_attempts_on_the_card(10, 1..=10, false).await;
}

#[tokio::test]
async fn the_review_card_hides_attempts_before_the_latest_ten_and_says_so() {
    assert_previous_attempts_on_the_card(11, 2..=11, true).await;
}

async fn assert_previous_attempts_on_the_card(
    previous: u16,
    shown: RangeInclusive<u16>,
    hidden: bool,
) {
    let app = TestApp::start().await;
    let assignment = assignment_of(&app.fixture.course, &app.fixture.teacher)
        .max_attempts(previous + 1)
        .published()
        .build();
    db::assignments::insert(&app.pool, &assignment)
        .await
        .unwrap();
    for used in 0..previous {
        let mut attempt = submission_of(&assignment, &app.fixture.student)
            .history(history().used(used).build())
            .content(format!("pull/{}", used + 1))
            .build();
        attempt
            .reject(
                &assignment,
                &app.fixture.course,
                &app.fixture.teacher,
                "Ещё раз",
                Utc::now(),
            )
            .unwrap();
        db::submissions::insert(&app.pool, &attempt).await.unwrap();
    }
    let pending = submission_of(&assignment, &app.fixture.student)
        .history(history().used(previous).build())
        .build();
    db::submissions::insert(&app.pool, &pending).await.unwrap();

    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, pending.id().as_uuid()),
        )
        .await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    let rejected = texts::submission_status(SubmissionStatus::Rejected);
    for number in 1..=previous {
        let attempt =
            texts::previous_attempt(number, rejected, &format!("pull/{number}"), Some("Ещё раз"));
        assert_eq!(
            sent.contains(&attempt),
            shown.contains(&number),
            "attempt {number}: {sent:#?}"
        );
    }
    assert_eq!(
        sent.contains(&texts::earlier_attempts_hidden(10)),
        hidden,
        "{sent:#?}"
    );
}

#[tokio::test]
async fn review_restarts_when_the_selected_score_exceeds_the_new_scale() {
    assert_review_recovers_from_grading_change(Some(5), Some(3)).await;
}

#[tokio::test]
async fn review_restarts_when_the_scale_grows_after_the_score_was_chosen() {
    assert_review_recovers_from_grading_change(Some(5), Some(10)).await;
}

#[tokio::test]
async fn review_restarts_when_scored_grading_becomes_pass_fail() {
    assert_review_recovers_from_grading_change(Some(5), None).await;
}

#[tokio::test]
async fn review_restarts_when_pass_fail_grading_starts_requiring_a_score() {
    assert_review_recovers_from_grading_change(None, Some(5)).await;
}

async fn assert_review_recovers_from_grading_change(
    initial_max: Option<u16>,
    new_max: Option<u16>,
) {
    let app = TestApp::start().await;
    let assignment = match initial_max {
        Some(max) => app.add_scored_assignment(max).await.id().as_uuid(),
        None => app.assignment(),
    };
    let submission = submission_id(
        &app.submit(assignment, app.student(), "pull/recovery")
            .await
            .created(),
    );
    let accept = format!("{}{submission}", callbacks::REVIEW_ACCEPT_PREFIX);
    let mut initial = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::NEXT_REVIEW, BOT_MESSAGE_ID),
        callback_update(3, app.teacher(), &accept, BOT_MESSAGE_ID),
    ];
    if initial_max.is_some() {
        initial.push(callback_update(
            4,
            app.teacher(),
            &format!("{}{submission}:4", callbacks::REVIEW_SCORE_PREFIX),
            BOT_MESSAGE_ID,
        ));
    }
    let updates = Arc::new(Mutex::new(initial));
    let polled_updates = updates.clone();
    let telegram = telegram_server(Vec::new()).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetUpdates"))
        .respond_with(move |request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let offset = body["offset"].as_i64().unwrap_or(0);
            let result: Vec<_> = polled_updates
                .lock()
                .unwrap()
                .iter()
                .filter(|update| update["update_id"].as_i64().unwrap() >= offset)
                .cloned()
                .collect();
            let empty = result.is_empty();
            let response =
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result}));
            if empty && body["timeout"].as_u64().unwrap_or(0) > 0 {
                response.set_delay(Duration::from_millis(20))
            } else {
                response
            }
        })
        .with_priority(1)
        .mount(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    assert!(
        wait_until(Duration::from_secs(5), || async {
            telegram
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| {
                    request.body_json::<Value>().ok().is_some_and(|body| {
                        body["text"] == texts::input_prompt(texts::REVIEW_COMMENT_PROMPT)
                    })
                })
        })
        .await,
        "the review did not reach the comment step"
    );

    app.change_grading(assignment, app.teacher(), new_max)
        .await
        .ok();
    let mut remaining = vec![
        message_update(5, app.teacher(), 2, "Старый комментарий"),
        callback_update(6, app.teacher(), &accept, BOT_MESSAGE_ID),
    ];
    let new_score = new_max.map(|max| max.min(4));
    if let Some(score) = new_score {
        remaining.push(callback_update(
            7,
            app.teacher(),
            &format!("{}{submission}:{score}", callbacks::REVIEW_SCORE_PREFIX),
            BOT_MESSAGE_ID,
        ));
    }
    remaining.push(message_update(
        8,
        app.teacher(),
        3,
        "Обновлённый комментарий",
    ));
    updates.lock().unwrap().extend(remaining);

    let result = wait_for_review_result(&app.pool, submission).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let (status, raw_score, _) =
        result.unwrap_or_else(|| panic!("review did not recover; requests:\n{requests}"));
    assert_eq!(status, "accepted");
    assert_eq!(raw_score, new_score.map(i16::try_from).transpose().unwrap());
    assert!(
        requests.contains(texts::REVIEW_GRADING_CHANGED),
        "{requests}"
    );
    let saved = hw_bot::db::submissions::find_by_id(
        &app.pool,
        hw_bot::domain::SubmissionId::from_uuid(submission),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        saved.review_comment().unwrap().as_str(),
        "Обновлённый комментарий"
    );
}

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
            &format!("{}{submission}:4", callbacks::REVIEW_SCORE_PREFIX),
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

    let (status, raw_score, work_score) =
        reviewed.unwrap_or_else(|| panic!("submission was not reviewed; requests:\n{requests}"));
    assert_eq!(status, "accepted");
    assert_eq!(raw_score, Some(4));
    assert_eq!(work_score, Some(4));
}

#[tokio::test]
async fn score_button_from_another_submission_is_stale() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let other_student = app.add_student("Second student").await;
    let first = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let second = submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            other_student.telegram_user_id().as_i64(),
            "pull/2",
        )
        .await
        .created(),
    );
    let score =
        |submission, value| format!("{}{submission}:{value}", callbacks::REVIEW_SCORE_PREFIX);
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::NEXT_REVIEW, BOT_MESSAGE_ID),
        callback_update(
            3,
            app.teacher(),
            &format!("{}{first}", callbacks::REVIEW_ACCEPT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(4, app.teacher(), &score(first, 3), BOT_MESSAGE_ID),
        message_update(5, app.teacher(), 2, "Первая"),
        callback_update(6, app.teacher(), callbacks::NEXT_REVIEW, BOT_MESSAGE_ID),
        callback_update(
            7,
            app.teacher(),
            &format!("{}{second}", callbacks::REVIEW_ACCEPT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(8, app.teacher(), &score(first, 1), BOT_MESSAGE_ID),
        callback_update(9, app.teacher(), &score(second, 4), BOT_MESSAGE_ID),
        message_update(10, app.teacher(), 3, "Вторая"),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let reviewed = wait_for_review_result(&app.pool, second).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    let (status, raw_score, work_score) =
        reviewed.unwrap_or_else(|| panic!("submission was not reviewed; requests:\n{requests}"));
    assert_eq!(status, "accepted");
    assert_eq!(raw_score, Some(4));
    assert_eq!(work_score, Some(4));
    assert!(
        requests.contains(texts::STALE_BUTTON),
        "the old score button was not rejected; requests:\n{requests}"
    );
}

#[tokio::test]
async fn review_confirmation_uses_the_grade_saved_after_a_deadline_change() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((1, 1, 4))).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_SCORE_PREFIX, format!("{submission}:4")),
        )
        .await;
    app.change_deadline(
        assignment.id().as_uuid(),
        app.teacher(),
        Utc::now() + ChronoDuration::days(1),
    )
    .await
    .ok();

    telegram.message(app.teacher(), "Хорошо");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let saved = hw_bot::db::submissions::find_by_id(
        &app.pool,
        hw_bot::domain::SubmissionId::from_uuid(submission),
    )
    .await
    .unwrap()
    .unwrap()
    .grade()
    .unwrap();
    assert_eq!(saved.work_score(), 4);
    assert_eq!(saved.penalty(), 0);
    assert!(telegram.messages(app.teacher()).iter().any(|message| {
        message["text"]
            .as_str()
            .is_some_and(|text| text.contains(&regular_grade(saved)))
    }));
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
        message_update(2, app.teacher(), 2, texts::BUTTON_REVIEW),
    ];
    let telegram = telegram_server(updates).await;
    let empty = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "chat_id": app.teacher(),
            "text": texts::REVIEW_QUEUE_EMPTY
        })))
        .and(|request: &wiremock::Request| has_callback_button(request, callbacks::REVIEW_HUB))
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
    app.review_with_score(submission, app.teacher(), "accept", out_of(4, 5), "Ок")
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

    wait_for_sent_message_count(&telegram, 3).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert!(
        requests.contains("Итого: 4 из 5"),
        "the student card carries the total; requests:\n{requests}"
    );
    assert!(
        requests.contains("Rust traits · 4 из 5"),
        "the drill-down carries the per-assignment score; requests:\n{requests}"
    );
}

#[tokio::test]
async fn the_student_card_shows_penalties_to_the_course_total() {
    let app = TestApp::start().await;
    let to_total = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let on_time = app.add_scored_assignment(10).await;
    for (assignment, score) in [(&to_total, 8), (&on_time, 6)] {
        let submission = submission_id(
            &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
                .await
                .created(),
        );
        app.review_with_score(submission, app.teacher(), "accept", out_of(score, 10), "Ок")
            .await
            .ok();
    }
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADES_PREFIX, app.course()))
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
    stop_bot(cancellation, &mut task).await;

    let header = texts::grades_header(app.fixture.student.display_name().as_str(), 14, 4, 10, 20);
    let sent = texts_sent(&telegram, app.teacher());
    let card = sent
        .iter()
        .find(|text| text.starts_with(&header))
        .unwrap_or_else(|| panic!("the card does not sum the penalties: {sent:#?}"));
    assert!(
        card.contains(&texts::grade_entry("To total", "8", 10, 4, false)),
        "{card}"
    );
    assert!(
        card.contains(&texts::grade_entry(
            on_time.title().as_str(),
            "6",
            10,
            0,
            false
        )),
        "{card}"
    );
}

#[tokio::test]
async fn a_resubmission_card_shows_the_previous_attempt_with_its_comment() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/first").await;
    app.review(first, app.teacher(), "reject", "Fix the tests")
        .await
        .ok();
    app.queued_submission(app.student(), "pull/second").await;
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let messages: Vec<String> = telegram
        .messages(app.teacher())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    let previous = messages
        .iter()
        .position(|text| text.contains("pull/first") && text.contains("Fix the tests"))
        .expect("the rejected attempt is shown with its comment");
    let current = messages
        .iter()
        .position(|text| text == "pull/second")
        .expect("the pending attempt is shown");
    assert!(previous < current);
    assert!(
        messages[..current]
            .iter()
            .any(|text| text.contains(app.fixture.subject.name().as_str()) && text.contains('2')),
        "the summary names the subject and the attempt number"
    );
}

#[tokio::test]
async fn a_long_previous_attempt_does_not_hide_the_decision_buttons() {
    let app = TestApp::start().await;
    let first = app
        .queued_submission(app.student(), &"x".repeat(USER_TEXT_MAX_LEN))
        .await;
    app.review(first, app.teacher(), "reject", "Fix the tests")
        .await
        .ok();
    let second = app.queued_submission(app.student(), "pull/second").await;
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/review");
    telegram
        .button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, second))
        .await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_failed_previous_attempt_does_not_hide_the_decision_buttons() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/first").await;
    app.review(first, app.teacher(), "reject", "Fix the tests")
        .await
        .ok();
    let second = app.queued_submission(app.student(), "pull/second").await;
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(|request: &wiremock::Request| {
            request.body_json::<Value>().is_ok_and(|body| {
                body["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("pull/first"))
            })
        })
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "ok": false,
            "error_code": 429,
            "description": "Too Many Requests: retry after 1",
            "parameters": {"retry_after": 1}
        })))
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/review");
    telegram
        .button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, second))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        telegram.messages(app.teacher()).iter().any(|message| {
            message["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(texts::PREVIOUS_ATTEMPTS_INCOMPLETE))
        }),
        "the reviewer is told that the history is incomplete"
    );
}

async fn add_trial(app: &TestApp) -> Assignment {
    let sandbox = app.add_sandbox_course().await;

    app.add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await
}

async fn submit_trial(app: &TestApp, trial: &Assignment, student: i64) -> Uuid {
    submission_id(
        &app.submit(trial.id().as_uuid(), student, "pull/trial")
            .await
            .created(),
    )
}

fn texts_sent(telegram: &TelegramScenario, user: i64) -> Vec<String> {
    telegram
        .messages(user)
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect()
}

async fn is_pending(app: &TestApp, submission: Uuid) -> bool {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .expect("read submission")
        .expect("submission exists")
        .is_pending_review()
}

#[tokio::test]
async fn the_sandbox_menu_item_lists_sandbox_submissions_with_bulk_buttons() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_OPEN_PREFIX, submission),
        )
        .await;
    telegram
        .button(app.teacher(), callbacks::SANDBOX_ACCEPT_ALL)
        .await;
    telegram
        .button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        texts_sent(&telegram, app.teacher())
            .iter()
            .any(|text| text.starts_with(&texts::sandbox_header(1)))
    );
}

#[tokio::test]
async fn an_empty_sandbox_is_reported_without_bulk_buttons() {
    let app = TestApp::start().await;
    add_trial(&app).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::SANDBOX_EMPTY.to_owned()));
    assert!(
        !sent_callbacks(&telegram.server)
            .await
            .iter()
            .any(|data| data == callbacks::SANDBOX_ACCEPT_ALL
                || data == callbacks::SANDBOX_REJECT_ALL)
    );
}

#[tokio::test]
async fn an_old_sandbox_button_opens_the_review_card_and_saves_the_decision() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    let open = telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_OPEN_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "/menu");
    telegram.flush(app.teacher()).await;

    telegram.click(app.teacher(), &open);
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_REJECT_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "Учебная сдача");
    let reviewed = wait_for_review_result(&app.pool, submission).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        reviewed.map(|(status, _, _)| status).as_deref(),
        Some("rejected")
    );
    let notifications: i64 = sqlx::query_scalar(
        "select count(*) from review_notification_outbox where submission_id = $1",
    )
    .bind(submission)
    .fetch_one(&app.pool)
    .await
    .expect("count notifications");
    assert_eq!(notifications, 1);
}

#[tokio::test]
async fn a_reviewed_sandbox_submission_does_not_open_again() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    let open = telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_OPEN_PREFIX, submission),
        )
        .await;
    app.review(submission, app.teacher(), "reject", "Вручную")
        .await
        .ok();

    telegram.click(app.teacher(), &open);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(sent.contains(&app_error_message(ErrorCode::SubmissionAlreadyReviewed).to_owned()));
    assert!(!sent.contains(&texts::SUBMISSION_HEADER.to_owned()));
}

#[tokio::test]
async fn the_home_screen_counts_the_sandbox_apart_from_the_queue() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    for name in ["First", "Second", "Third"] {
        let trainee = app.add_student(name).await;
        submit_trial(&app, &trial, trainee.telegram_user_id().as_i64()).await;
    }
    app.queued_submission(app.student(), "pull/regular").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/menu");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let greeting = texts::home_greeting(app.fixture.teacher.display_name().as_str());
    let home = texts_sent(&telegram, app.teacher())
        .into_iter()
        .find(|text| text.starts_with(&greeting))
        .expect("the home screen is shown");
    assert!(home.contains(&texts::review_queue_line(1)), "{home}");
    assert!(home.contains(&texts::home_sandbox_line(3)), "{home}");
}

#[tokio::test]
async fn a_bulk_button_from_an_old_sandbox_screen_is_stale() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    let old = telegram
        .button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .button_after(app.teacher(), callbacks::SANDBOX_REJECT_ALL, old.message_id)
        .await;

    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(sent.contains(&texts::STALE_BUTTON.to_owned()));
    assert!(
        !sent.contains(&texts::input_prompt(&texts::bulk_review_prompt(
            ReviewDecision::Reject,
            1
        )))
    );
    assert!(is_pending(&app, submission).await);
}

#[tokio::test]
async fn a_bulk_button_with_nothing_left_skips_the_comment_step() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    let reject_all = telegram
        .button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;
    app.review(submission, app.teacher(), "accept", "Вручную")
        .await
        .ok();

    telegram.click(app.teacher(), &reject_all);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(sent.contains(&texts::SANDBOX_NOTHING_TO_DECIDE.to_owned()));
    assert!(
        !sent.contains(&texts::input_prompt(&texts::bulk_review_prompt(
            ReviewDecision::Reject,
            1
        )))
    );
}

#[tokio::test]
async fn the_accept_all_prompt_warns_before_asking_for_the_comment() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .click_button(app.teacher(), callbacks::SANDBOX_ACCEPT_ALL)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        texts_sent(&telegram, app.teacher()).contains(&texts::input_prompt(
            &texts::bulk_review_prompt(ReviewDecision::Accept, 1)
        ))
    );
}

#[tokio::test]
async fn rejecting_all_in_the_bot_rejects_the_sandbox_and_returns_the_menu() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .click_button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;

    telegram.message(app.teacher(), "Учебная сдача");
    let reviewed = wait_for_review_result(&app.pool, submission).await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        reviewed.map(|(status, _, _)| status).as_deref(),
        Some("rejected")
    );
    let saved = texts::bulk_review_saved(ReviewDecision::Reject, 1);
    let confirmation = telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| message["text"] == saved.as_str())
        .expect("the bulk decision is confirmed");
    let menu = confirmation["reply_markup"]["keyboard"].to_string();
    assert!(menu.contains(texts::BUTTON_SANDBOX), "{menu}");
}

#[tokio::test]
async fn cancelling_the_bulk_comment_changes_nothing() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let submission = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .click_button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;

    telegram.message(app.teacher(), "/cancel");
    telegram.message(app.teacher(), "Учебная сдача");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::ACTION_CANCELLED.to_owned()));
    assert!(is_pending(&app, submission).await);
}

#[tokio::test]
async fn a_failed_bulk_confirmation_does_not_repeat_the_decision() {
    let app = TestApp::start().await;
    let trial = add_trial(&app).await;
    let first = submit_trial(&app, &trial, app.student()).await;
    let telegram = TelegramScenario::new().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "text": texts::bulk_review_saved(ReviewDecision::Reject, 1)
        })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400, "description": "Bad Request: injected failure",
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&telegram.server)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .click_button(app.teacher(), callbacks::SANDBOX_REJECT_ALL)
        .await;
    telegram.message(app.teacher(), "Учебная сдача");
    wait_for_review_result(&app.pool, first)
        .await
        .expect("the bulk decision is committed");
    let trainee = app.add_student("Late commit").await;
    let late = submission_of(&trial, &trainee)
        .submitted_at(Utc::now() - ChronoDuration::minutes(10))
        .build();
    db::submissions::insert(&app.pool, &late)
        .await
        .expect("insert a submission committed after the decision");

    telegram.message(app.teacher(), "Учебная сдача");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(is_pending(&app, late.id().as_uuid()).await);
}

async fn submitted(app: &TestApp, assignment: &Assignment) -> Uuid {
    submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/late")
            .await
            .created(),
    )
}

fn message_text(telegram: &TelegramScenario, user: i64, message_id: i32) -> String {
    telegram
        .messages(user)
        .into_iter()
        .find(|message| message["message_id"] == message_id)
        .and_then(|message| message["text"].as_str().map(str::to_owned))
        .expect("the message was sent")
}

#[tokio::test]
async fn a_late_card_shows_the_maximum_left_after_a_penalty_to_the_work_score() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("Late work", 10, 4, DeductedFrom::WorkScore)
        .await;
    submitted(&app, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/review");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let late = assignment
        .late_policy()
        .unwrap()
        .late_penalty(1)
        .expect("the submission is one period late");
    let line = texts::late_submission_line(late, 10);
    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter().any(|text| text.contains(&line)),
        "the card does not show the maximum: {sent:#?}"
    );
}

#[tokio::test]
async fn changing_the_decision_after_a_score_saves_a_rejection_without_a_score() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("Late work", 10, 4, DeductedFrom::WorkScore)
        .await;
    let submission = submitted(&app, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_SCORE_PREFIX, format!("{submission}:8")),
        )
        .await;

    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_CHANGE_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "Это уже не комментарий");
    telegram.flush(app.teacher()).await;
    assert!(is_pending(&app, submission).await);
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_REJECT_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "Нужны тесты");
    let reviewed = wait_for_review_result(&app.pool, submission).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(reviewed, Some(("rejected".to_owned(), None, None)));
    let sent = texts_sent(&telegram, app.teacher());
    assert!(sent.contains(&texts::CHOOSE_REVIEW_DECISION.to_owned()));
    let reset = telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| message["text"] == texts::REVIEW_DECISION_RESET)
        .expect("the decision is reset");
    let menu = reset["reply_markup"]["keyboard"].to_string();
    assert!(menu.contains(texts::BUTTON_REVIEW), "{menu}");
    let saved = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.review_comment().unwrap().as_str(), "Нужны тесты");
}

#[tokio::test]
async fn a_change_button_after_the_review_is_saved_is_stale() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    let change = telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_CHANGE_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "Зачтено");
    wait_for_review_result(&app.pool, submission)
        .await
        .expect("the review is saved");

    telegram.click(app.teacher(), &change);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(sent.contains(&texts::STALE_BUTTON.to_owned()), "{sent:#?}");
    assert!(!sent.contains(&texts::REVIEW_DECISION_RESET.to_owned()));
    let saved = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.review_comment().unwrap().as_str(), "Зачтено");
}

#[tokio::test]
async fn a_change_after_another_teacher_reviewed_the_submission_ends_the_review() {
    let app = TestApp::start().await;
    let colleague = app.add_course_teacher("Colleague").await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    app.review(
        submission,
        colleague.telegram_user_id().as_i64(),
        "reject",
        "Решение коллеги",
    )
    .await
    .ok();

    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_CHANGE_PREFIX, submission),
        )
        .await;
    telegram.message(app.teacher(), "Поздний комментарий");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.contains(&app_error_message(ErrorCode::SubmissionAlreadyReviewed).to_owned()),
        "{sent:#?}"
    );
    let greeting = texts::home_greeting(app.fixture.teacher.display_name().as_str());
    assert!(
        sent.iter().any(|text| text.starts_with(&greeting)),
        "{sent:#?}"
    );
    assert!(sent.contains(&texts::USE_THE_MENU.to_owned()), "{sent:#?}");
    let saved = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.review_comment().unwrap().as_str(), "Решение коллеги");
}

#[tokio::test]
async fn the_confirmation_shows_the_deduction_saved_after_a_policy_change() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("Late work", 10, 4, DeductedFrom::CourseTotal)
        .await;
    let submission = submitted(&app, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/review");
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, submission),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_SCORE_PREFIX, format!("{submission}:8")),
        )
        .await;
    let change = telegram
        .button(
            app.teacher(),
            &cb(callbacks::REVIEW_CHANGE_PREFIX, submission),
        )
        .await;
    let preview = message_text(&telegram, app.teacher(), change.message_id);
    app.post_as(
        &format!("/assignments/{}/grading", assignment.id().as_uuid()),
        app.teacher(),
        json!({
            "max_score": 10, "late_period_days": 7, "late_penalty_per_period": 4, "late_max_periods": 4,
            "late_deducted_from": "work_score",
        }),
    )
    .await
    .ok();

    telegram.message(app.teacher(), "Хорошо");
    wait_for_review_result(&app.pool, submission)
        .await
        .expect("the review is saved");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let saved = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap()
        .grade()
        .unwrap();
    assert_eq!(saved.deducted_from(), Some(DeductedFrom::WorkScore));
    assert_eq!(saved.work_score(), 4);
    let line = regular_grade(saved);
    assert!(!preview.contains(&line), "{preview}");
    let sent = texts_sent(&telegram, app.teacher());
    let confirmation = sent
        .iter()
        .find(|text| text.starts_with(texts::REVIEW_SAVED))
        .unwrap_or_else(|| panic!("the review is not confirmed: {sent:#?}"));
    assert!(confirmation.contains(&line), "{confirmation}");
}
