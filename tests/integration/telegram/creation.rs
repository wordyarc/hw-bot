use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use hw_bot::bot::{callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;

#[tokio::test]
async fn teacher_creates_a_draft_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(7);
    let deadline_input = deadline_input(deadline);
    let expected_deadline = expected_deadline(&deadline_input);
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
        message_update(5, app.teacher(), 2, "Telegram draft"),
        message_update(
            6,
            app.teacher(),
            3,
            "Solve the task and send a pull request.",
        ),
        message_update(7, app.teacher(), 4, "3"),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, 5),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            9,
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "none"),
            BOT_MESSAGE_ID,
        ),
        message_update(10, app.teacher(), 5, &deadline_input),
        callback_update(11, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Telegram draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let callbacks = sent_callbacks(&telegram).await;
    telegram.verify().await;

    let created = created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    let CreatedAssignment {
        id,
        status,
        body,
        max_attempts,
        max_score,
        deadline_at: stored_deadline,
    } = created;
    assert_eq!(status, "draft");
    assert_eq!(body, "Solve the task and send a pull request.");
    assert_eq!(max_attempts, 3);
    assert_eq!(max_score, Some(5), "the scale came from a button");
    assert_eq!(stored_deadline, expected_deadline);
    assert!(callbacks.contains(&format!("{}{}", callbacks::COURSE_PREFIX, app.course())));
    assert!(callbacks.contains(&format!("{}{}", callbacks::CREATE_PREFIX, app.course())));
    assert!(callbacks.contains(&format!("{}{id}", callbacks::PUBLISH_PREFIX)));
    assert!(callbacks.iter().all(|callback| callback.len() <= 64));
}

#[tokio::test]
async fn a_prompt_hides_the_menu_and_the_finished_step_brings_it_back() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("{}{}", callbacks::SUBMIT_PREFIX, app.assignment()),
            BOT_MESSAGE_ID,
        ),
        message_update(3, app.student(), 2, "https://example.test/pull/1"),
    ];
    let telegram = telegram_server(updates).await;
    let prompt = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": texts::input_prompt(texts::SUBMISSION_PROMPT),
            "reply_markup": { "remove_keyboard": true }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let restored = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": texts::submission_queued(),
            "reply_markup": {
                "keyboard": [
                    [{ "text": "📚 Мои задания" }, { "text": "⏳ Очередь проверки" }],
                    [{ "text": "🗄 Архив" }, { "text": "👤 Профиль" }],
                    [{ "text": "❓ Помощь" }]
                ],
                "is_persistent": true,
                "resize_keyboard": true,
                "input_field_placeholder": "Выберите пункт меню"
            }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let asked = tokio::time::timeout(Duration::from_secs(5), prompt.wait_until_satisfied()).await;
    let returned =
        tokio::time::timeout(Duration::from_secs(5), restored.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    asked.unwrap_or_else(|_| panic!("the prompt did not hide the menu; requests:\n{requests}"));
    returned.unwrap_or_else(|_| panic!("the menu did not come back; requests:\n{requests}"));
}

#[tokio::test]
async fn teacher_picks_a_deadline_with_buttons_instead_of_typing_a_date() {
    let app = TestApp::start().await;
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
        message_update(5, app.teacher(), 2, "Deadline in a week"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        callback_update(
            7,
            app.teacher(),
            &cb(callbacks::MAX_ATTEMPTS_PREFIX, 3),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, "passfail"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            9,
            app.teacher(),
            &cb(callbacks::DEADLINE_DAYS_PREFIX, 7),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Deadline in a week").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    assert!(
        requests.contains(&format!(
            r#""callback_data":"{}""#,
            cb(callbacks::MAX_ATTEMPTS_PREFIX, 3)
        )),
        "the keyboard must send exactly what the handler matches; requests:\n{requests}"
    );
    let created = created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    let (status, max_attempts, deadline) =
        (created.status, created.max_attempts, created.deadline_at);
    assert_eq!(status, "draft");
    assert_eq!(
        max_attempts, 3,
        "attempts came from a button, not from text"
    );
    assert_eq!(created.max_score, None, "the skipped scale means pass/fail");
    let local = deadline.with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap());
    assert_eq!(
        local.date_naive(),
        (Utc::now() + chrono::Duration::days(7))
            .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
            .date_naive(),
        "«через неделю» is a week from today in the configured timezone"
    );
    assert_eq!(
        local.format("%H:%M").to_string(),
        "23:59",
        "presets expire at the end of the local day"
    );
}

#[tokio::test]
async fn teacher_picks_a_deadline_from_the_calendar() {
    let app = TestApp::start().await;
    let picked = (Utc::now() + chrono::Duration::days(3))
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .date_naive();
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
        message_update(5, app.teacher(), 2, "Calendar draft"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        callback_update(
            7,
            app.teacher(),
            &cb(callbacks::MAX_ATTEMPTS_PREFIX, 1),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, "passfail"),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            9,
            app.teacher(),
            callbacks::DEADLINE_CALENDAR,
            BOT_MESSAGE_ID,
        ),
        callback_update(
            10,
            app.teacher(),
            &format!(
                "{}{}",
                callbacks::CALENDAR_PICK_PREFIX,
                picked.format("%Y-%m-%d")
            ),
            BOT_MESSAGE_ID,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Calendar draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let deadline = created
        .unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"))
        .deadline_at;
    assert_eq!(
        deadline
            .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
            .date_naive(),
        picked
    );
    assert!(
        requests.contains("EditMessageReplyMarkup"),
        "the calendar replaces the keyboard in place; requests:\n{requests}"
    );
}

#[tokio::test]
async fn teacher_types_a_scale_that_has_no_preset_button() {
    let app = TestApp::start().await;
    let deadline_input = deadline_input(Utc::now() + chrono::Duration::days(7));
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
        message_update(5, app.teacher(), 2, "Custom scale draft"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        message_update(7, app.teacher(), 4, "2"),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, "manual"),
            BOT_MESSAGE_ID,
        ),
        message_update(9, app.teacher(), 5, "7"),
        callback_update(
            10,
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "none"),
            BOT_MESSAGE_ID,
        ),
        message_update(11, app.teacher(), 6, &deadline_input),
        callback_update(12, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Custom scale draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let created = created.unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"));
    assert_eq!(created.max_score, Some(7));
}

#[tokio::test]
async fn teacher_picks_a_late_policy_preset_during_creation() {
    let app = TestApp::start().await;
    let deadline_input = deadline_input(Utc::now() + chrono::Duration::days(7));
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
        message_update(5, app.teacher(), 2, "Late policy draft"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        message_update(7, app.teacher(), 4, "2"),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, 5),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            9,
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "7:1:4"),
            BOT_MESSAGE_ID,
        ),
        message_update(10, app.teacher(), 5, &deadline_input),
        callback_update(11, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "Late policy draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    let id = created
        .unwrap_or_else(|| panic!("draft was not created; requests:\n{requests}"))
        .id;
    let policy = sqlx::query_as::<_, (Option<i16>, Option<i16>, Option<i16>)>(
        "select late_period_days, late_penalty, late_max_periods from assignments where id = $1",
    )
    .bind(id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(policy, (Some(7), Some(1), Some(4)));
}

#[tokio::test]
async fn a_pass_fail_draft_is_never_asked_about_late_penalties() {
    let app = TestApp::start().await;
    let deadline_input = deadline_input(Utc::now() + chrono::Duration::days(7));
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
        message_update(5, app.teacher(), 2, "No penalties draft"),
        message_update(6, app.teacher(), 3, "Send a pull request."),
        message_update(7, app.teacher(), 4, "2"),
        callback_update(
            8,
            app.teacher(),
            &cb(callbacks::GRADING_PREFIX, "passfail"),
            BOT_MESSAGE_ID,
        ),
        message_update(9, app.teacher(), 5, &deadline_input),
        callback_update(10, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let created = wait_for_created_assignment(&app.pool, "No penalties draft").await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    telegram.verify().await;

    assert!(
        created.is_some(),
        "the deadline followed the scale directly; requests:\n{requests}"
    );
    assert!(
        !requests.contains(texts::LATE_POLICY_QUESTION),
        "requests:\n{requests}"
    );
}

#[tokio::test]
async fn teacher_changes_an_assignment_and_grants_attempts_through_telegram() {
    let app = TestApp::start().await;
    let deadline = Utc::now() + chrono::Duration::days(14);
    let deadline_input = deadline_input(deadline);
    let expected_deadline = expected_deadline(&deadline_input);
    let assignment = app.assignment();
    let student = app.fixture.student.id().as_uuid();
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
            &format!("{}{assignment}", callbacks::ARCHIVE_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            6,
            app.teacher(),
            &format!("{}{assignment}", callbacks::PUBLISH_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            7,
            app.teacher(),
            &format!("{}{assignment}", callbacks::DEADLINE_PREFIX),
            BOT_MESSAGE_ID,
        ),
        message_update(8, app.teacher(), 2, &deadline_input),
        callback_update(
            9,
            app.teacher(),
            &format!("{}{assignment}", callbacks::ATTEMPTS_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            10,
            app.teacher(),
            &format!("{}{student}", callbacks::STUDENT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        message_update(11, app.teacher(), 3, "2"),
    ];
    let telegram = telegram_server(updates).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let granted = wait_for_extra_attempts(&app.pool, assignment, student, 2).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let callbacks = sent_callbacks(&telegram).await;
    telegram.verify().await;

    assert!(
        granted,
        "extra attempts were not granted; requests:\n{requests}"
    );
    let row = sqlx::query_as::<_, (String, DateTime<Utc>, i64)>(
        "select status, deadline_at, revision from assignments where id = $1",
    )
    .bind(assignment)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(row.0, "published");
    assert_eq!(row.1, expected_deadline);
    assert_eq!(
        row.2, 3,
        "archive, re-publication and deadline each write once"
    );
    for expected in [
        format!("{}{}", callbacks::COURSE_PREFIX, app.course()),
        format!("{}{assignment}", callbacks::MANAGE_PREFIX),
        format!("{}{assignment}", callbacks::ARCHIVE_PREFIX),
        format!("{}{assignment}", callbacks::PUBLISH_PREFIX),
        format!("{}{assignment}", callbacks::DEADLINE_PREFIX),
        format!("{}{assignment}", callbacks::ATTEMPTS_PREFIX),
        format!("{}{student}", callbacks::STUDENT_PREFIX),
    ] {
        assert!(callbacks.contains(&expected), "missing callback {expected}");
    }
    assert!(callbacks.iter().all(|callback| callback.len() <= 64));
}
