use std::time::Duration;

use chrono::{DateTime, Datelike, FixedOffset, Months, Utc};
use hw_bot::bot::{callbacks, texts};
use hw_bot::domain::{DeductedFrom, LATEST_TERM_YEAR, MAX_PENALTY_PER_PERIOD, StudentGroup};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, body_partial_json, method, path};

use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;
use crate::harness::{TestApp, insert_assignment, wall_clock_assignment};

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
        callback_update(11, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
        callback_update(12, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
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
        .and(body_partial_json(json!({
            "chat_id": app.student(),
            "text": texts::submission_queued()
        })))
        .and(|request: &wiremock::Request| has_reply_button(request, texts::BUTTON_ASSIGNMENTS))
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
        callback_update(10, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
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
        local.time(),
        chrono::NaiveTime::MIN,
        "presets end at the local midnight that closes the day"
    );
    assert_eq!(
        local.date_naive().pred_opt().unwrap(),
        (Utc::now() + chrono::Duration::days(7))
            .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
            .date_naive(),
        "«через неделю» is a week from today in the configured timezone"
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
        callback_update(11, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
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
    let local = deadline.with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap());
    assert_eq!(local.time(), chrono::NaiveTime::MIN);
    assert_eq!(
        local.date_naive().pred_opt().unwrap(),
        picked,
        "a picked day closes at the midnight that ends it"
    );
    assert!(
        requests.contains("EditMessageText"),
        "the calendar replaces the keyboard in place; requests:\n{requests}"
    );
}

async fn open_deadline_calendar(app: &TestApp, telegram: &TelegramScenario) {
    let teacher = app.teacher();
    telegram.message(teacher, "/courses");
    telegram
        .click_button(teacher, &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::MANAGE_PREFIX, app.assignment()))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::DEADLINE_PREFIX, app.assignment()))
        .await;
    telegram
        .click_button(teacher, callbacks::TERMS_DEADLINE)
        .await;
    telegram
        .click_button(teacher, callbacks::DEADLINE_CALENDAR)
        .await;
}

#[tokio::test]
async fn the_back_button_under_the_calendar_returns_to_the_previous_step() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_deadline_calendar(&app, &telegram).await;

    let back = telegram
        .click_button(app.teacher(), callbacks::TERMS_BACK)
        .await;
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(save.message_id, back.message_id);
}

#[tokio::test]
async fn calendar_month_navigation_keeps_deadline_selection_active() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_deadline_calendar(&app, &telegram).await;
    let today = Utc::now()
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .date_naive();
    let next_month = today
        .with_day(1)
        .unwrap()
        .checked_add_months(Months::new(1))
        .unwrap();
    let month = telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::CALENDAR_MONTH_PREFIX, next_month.format("%Y-%m")),
        )
        .await;
    let pick = telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::CALENDAR_PICK_PREFIX, next_month),
        )
        .await;
    assert_eq!(month.message_id, pick.message_id);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let details = app
        .get_as(
            &format!("/me/assignments/{}", app.assignment()),
            app.student(),
        )
        .await
        .ok();
    let actual: DateTime<Utc> =
        serde_json::from_value(details["assignment"]["deadline_at"].clone()).unwrap();
    assert_eq!(
        actual,
        expected_deadline(&next_month.format("%d.%m.%Y").to_string())
    );
}

#[tokio::test]
async fn manual_deadline_input_invalidates_the_previous_inline_keyboard() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_deadline_calendar(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), callbacks::DEADLINE_OPTIONS)
        .await;
    let preset = telegram
        .button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::DEADLINE_MANUAL)
        .await;
    let route = format!("/me/assignments/{}", app.assignment());
    let before = app.get_as(&route, app.student()).await.ok();
    telegram.click(app.teacher(), &preset);
    telegram.flush(app.teacher()).await;
    let after = app.get_as(&route, app.student()).await.ok();
    assert_eq!(
        before["assignment"]["deadline_at"],
        after["assignment"]["deadline_at"]
    );

    let input = deadline_input(Utc::now() + chrono::Duration::days(40));
    telegram.message(app.teacher(), &input);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let details = app.get_as(&route, app.student()).await.ok();
    let actual: DateTime<Utc> =
        serde_json::from_value(details["assignment"]["deadline_at"].clone()).unwrap();
    assert_eq!(actual, expected_deadline(&input));
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
        callback_update(12, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
        callback_update(13, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
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
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Late policy draft").await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(7));
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    telegram.message(app.teacher(), &deadline);
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 1))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::FILE_FINISH)
        .await;
    telegram.flush(app.teacher()).await;
    let created = wait_for_created_assignment(&app.pool, "Late policy draft")
        .await
        .unwrap();
    stop_bot(cancellation, &mut task).await;
    let stored = hw_bot::db::assignments::find_by_id(
        &app.pool,
        hw_bot::domain::AssignmentId::from_uuid(created.id),
    )
    .await
    .unwrap()
    .unwrap();
    let policy = stored.late_policy().unwrap();
    assert_eq!(policy.period_days(), 7);
    assert_eq!(policy.penalty_per_period(), 1);
    assert_eq!(policy.max_periods(), 4);
    assert_eq!(
        stored.closes_at(),
        expected_deadline(&deadline) + chrono::Duration::weeks(4)
    );
}

#[tokio::test]
async fn a_draft_with_a_penalty_to_the_course_total_keeps_its_deduction() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Course total draft").await;
    telegram
        .click_button(
            app.teacher(),
            &cb(
                callbacks::LATE_POLICY_PREFIX,
                DeductedFrom::CourseTotal.as_str(),
            ),
        )
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 2))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    let summary = summary_text(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let created = wait_for_created_assignment(&app.pool, "Course total draft")
        .await
        .unwrap();
    let policy = stored_assignment(&app, created.id)
        .await
        .late_policy()
        .unwrap();
    assert_eq!(policy.deducted_from(), DeductedFrom::CourseTotal);
    assert_eq!((policy.penalty_per_period(), policy.max_periods()), (2, 4));
    assert!(
        summary.contains(&texts::deducted_from_line(DeductedFrom::CourseTotal)),
        "{summary}"
    );
    assert!(summary.contains(texts::SCHEDULE_TOTAL_HEADER), "{summary}");
}

#[tokio::test]
async fn the_terms_edit_names_the_deduction_and_changes_only_the_deduction() {
    let app = TestApp::start().await;
    let assignment = app
        .add_late_assignment("Course total", 10, 2, DeductedFrom::CourseTotal)
        .await;
    let id = assignment.id().as_uuid();
    let original = stored_assignment(&app, id).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;

    let policy = telegram
        .button_with_text(app.teacher(), texts::BUTTON_POLICY_COURSE_TOTAL)
        .await;
    telegram.click(app.teacher(), &policy);
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(
                callbacks::LATE_POLICY_PREFIX,
                DeductedFrom::WorkScore.as_str(),
            ),
        )
        .await;
    telegram
        .button_with_text(app.teacher(), texts::BUTTON_POLICY_WORK_SCORE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let saved = stored_assignment(&app, id).await;
    let before = original.late_policy().unwrap();
    let after = saved.late_policy().unwrap();
    assert_eq!(after.deducted_from(), DeductedFrom::WorkScore);
    assert_eq!(
        (
            after.period_days(),
            after.penalty_per_period(),
            after.max_periods()
        ),
        (
            before.period_days(),
            before.penalty_per_period(),
            before.max_periods()
        )
    );
    assert_eq!(saved.deadline_at(), original.deadline_at());
}

async fn open_scored_creation(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), title);
    telegram.message(app.teacher(), "Send a pull request.");
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADING_PREFIX, 5))
        .await;
}

#[tokio::test]
async fn invalid_period_count_keeps_the_draft_and_rejects_the_previous_deadline_keyboard() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Retry closing").await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    let old_date = telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 2))
        .await;
    telegram.click(app.teacher(), &old_date);
    telegram.message(app.teacher(), "101");
    telegram.flush(app.teacher()).await;
    let requests = recorded_requests(&telegram.server).await;
    assert!(requests.contains(texts::STALE_BUTTON));
    assert!(requests.contains(&texts::periods_question(7)));
    telegram.message(app.teacher(), "3");
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::FILE_FINISH)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let created = wait_for_created_assignment(&app.pool, "Retry closing")
        .await
        .unwrap();
    let saved = stored_assignment(&app, created.id).await;
    assert_eq!(saved.max_attempts(), 2);
    assert_eq!(saved.late_policy().unwrap().penalty_per_period(), 2);
    assert_eq!(
        saved.closes_at(),
        saved.deadline_at() + chrono::Duration::weeks(3)
    );
}

#[tokio::test]
async fn a_custom_penalty_period_and_amount_are_entered_separately() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Custom penalty").await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::PENALTY_PERIOD)
        .await;
    telegram.message(app.teacher(), "14");
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, "manual"))
        .await;
    telegram.message(app.teacher(), "3");
    telegram.flush(app.teacher()).await;
    telegram.message(app.teacher(), "3");
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::FILE_FINISH)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let created = wait_for_created_assignment(&app.pool, "Custom penalty")
        .await
        .unwrap();
    let policy = stored_assignment(&app, created.id)
        .await
        .late_policy()
        .unwrap();
    assert_eq!((policy.period_days(), policy.penalty_per_period()), (14, 3));
}

async fn stored_assignment(app: &TestApp, id: uuid::Uuid) -> hw_bot::domain::Assignment {
    hw_bot::db::assignments::find_by_id(&app.pool, hw_bot::domain::AssignmentId::from_uuid(id))
        .await
        .unwrap()
        .unwrap()
}

async fn draft_count(app: &TestApp, title: &str) -> usize {
    let assignments = app
        .get_as(
            &format!("/teaching-courses/{}/assignments", app.course()),
            app.teacher(),
        )
        .await
        .ok();
    assignments
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["title"] == title)
        .count()
}

async fn open_penalty_step(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    open_scored_creation(app, telegram, title).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
}

async fn open_periods_step(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    open_penalty_step(app, telegram, title).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 1))
        .await;
}

async fn summary_text(app: &TestApp, telegram: &TelegramScenario) -> String {
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| message["message_id"] == save.message_id)
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn all_button_steps_reuse_one_message_and_wait_for_explicit_save() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "One screen").await;
    let mode = telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    let date = telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    let penalty = telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 1))
        .await;
    let duration = telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    assert_eq!(
        [
            mode.message_id,
            date.message_id,
            penalty.message_id,
            duration.message_id
        ],
        [save.message_id; 4]
    );
    assert_eq!(draft_count(&app, "One screen").await, 0);
    assert!(
        summary_text(&app, &telegram)
            .await
            .contains(texts::SCHEDULE_HEADER)
    );
    telegram.click(app.teacher(), &save);
    telegram.flush(app.teacher()).await;
    assert_eq!(draft_count(&app, "One screen").await, 1);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn choosing_a_duration_date_requires_confirmation_of_rounded_periods() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Rounded duration").await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(7));
    telegram.message(app.teacher(), &deadline);
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 1))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DURATION_DATE)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::DEADLINE_CALENDAR)
        .await;
    let wanted = expected_deadline(&deadline) + chrono::Duration::days(10);
    let local = wanted
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .date_naive();
    let calendar_start = (expected_deadline(&deadline) + chrono::Duration::days(1))
        .with_timezone(&FixedOffset::east_opt(10 * 3600).unwrap())
        .date_naive();
    if local.with_day(1) != calendar_start.with_day(1) {
        telegram
            .click_button(
                app.teacher(),
                &cb(callbacks::CALENDAR_MONTH_PREFIX, local.format("%Y-%m")),
            )
            .await;
    }
    telegram
        .click_button(app.teacher(), &cb(callbacks::CALENDAR_PICK_PREFIX, local))
        .await;
    telegram
        .button(app.teacher(), callbacks::TERMS_DURATION_APPLY)
        .await;
    assert_eq!(draft_count(&app, "Rounded duration").await, 0);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DURATION_APPLY)
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    let created = wait_for_created_assignment(&app.pool, "Rounded duration")
        .await
        .unwrap();
    let saved = stored_assignment(&app, created.id).await;
    assert_eq!(saved.late_policy().unwrap().max_periods(), 2);
    assert_eq!(
        saved.closes_at(),
        expected_deadline(&deadline) + chrono::Duration::days(14)
    );
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn typed_answers_leave_no_live_buttons_on_the_creation_questions() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();
    telegram.message(teacher, "/courses");
    telegram
        .click_button(teacher, &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(teacher, "Typed answers");
    telegram.message(teacher, "Send a pull request.");
    let attempts = telegram
        .button(teacher, &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram.message(teacher, "2");
    let scale = telegram
        .click_button(teacher, &cb(callbacks::GRADING_PREFIX, "manual"))
        .await;
    telegram.message(teacher, "7");
    telegram
        .button(teacher, &cb(callbacks::LATE_POLICY_PREFIX, "none"))
        .await;
    stop_bot(cancellation, &mut task).await;

    for question in [attempts.message_id, scale.message_id] {
        let message = telegram
            .messages(teacher)
            .into_iter()
            .find(|message| message["message_id"] == question)
            .unwrap();
        assert!(message.get("reply_markup").is_none(), "{message:#?}");
    }
}

#[tokio::test]
async fn manual_input_removes_the_old_buttons_and_cancel_preserves_the_draft() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Keep my input").await;
    let old = telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, "manual"))
        .await;
    let previous = telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| message["message_id"] == old.message_id)
        .unwrap();
    assert!(previous.get("reply_markup").is_none());
    telegram.message(app.teacher(), "/cancel");
    telegram.flush(app.teacher()).await;
    let duration = telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    assert_ne!(duration.message_id, old.message_id);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    let created = wait_for_created_assignment(&app.pool, "Keep my input")
        .await
        .unwrap();
    assert_eq!(created.body, "Send a pull request.");
    assert_eq!(created.max_attempts, 2);
    assert_eq!(created.max_score, Some(5));
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn tapping_create_again_on_the_title_step_restarts_without_a_discard_notice() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    let create = cb(callbacks::CREATE_PREFIX, app.course());

    telegram.click_button(app.teacher(), &create).await;
    telegram.click_button(app.teacher(), &create).await;
    stop_bot(cancellation, &mut task).await;

    let sent = telegram.messages(app.teacher());
    assert!(
        !sent
            .iter()
            .any(|message| message["text"] == texts::DRAFT_DISCARDED),
        "{sent:#?}"
    );
    let prompts = sent
        .iter()
        .filter(|message| message["text"] == texts::input_prompt(texts::ASSIGNMENT_TITLE_PROMPT))
        .count();
    assert_eq!(prompts, 2);
}

#[tokio::test]
async fn cancelling_the_first_terms_step_exits_with_a_message_and_menu() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_scored_creation(&app, &telegram, "Discard first step").await;
    telegram.message(app.teacher(), "/cancel");
    telegram.flush(app.teacher()).await;
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|message| message["text"] == texts::DRAFT_DISCARDED
                && message["reply_markup"]["keyboard"].is_array())
    );
    assert_eq!(draft_count(&app, "Discard first step").await, 0);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_bottom_menu_misclick_can_be_cancelled_without_losing_input() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Menu misclick").await;
    telegram.message(app.teacher(), texts::BUTTON_COURSES);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_STAY)
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 2))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    assert_eq!(draft_count(&app, "Menu misclick").await, 1);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn confirmed_menu_exit_discards_the_draft_and_opens_the_destination() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Confirmed exit").await;
    telegram.message(app.teacher(), texts::BUTTON_COURSES);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_EXIT)
        .await;
    assert!(
        recorded_requests(&telegram.server)
            .await
            .contains(texts::DRAFT_DISCARDED)
    );
    telegram
        .button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    assert_eq!(draft_count(&app, "Confirmed exit").await, 0);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn navigation_command_discards_the_draft_without_a_confirmation_question() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Command exit").await;
    telegram.message(app.teacher(), "/courses");
    telegram.flush(app.teacher()).await;
    let requests = recorded_requests(&telegram.server).await;
    assert!(requests.contains(texts::DRAFT_DISCARDED));
    assert!(!requests.contains(texts::TERMS_EXIT_QUESTION));
    assert_eq!(draft_count(&app, "Command exit").await, 0);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn old_navigation_button_discards_the_draft_without_a_confirmation_question() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Inline exit").await;
    telegram
        .click_button(app.teacher(), callbacks::MENU_COURSES)
        .await;
    let requests = recorded_requests(&telegram.server).await;
    assert!(requests.contains(texts::DRAFT_DISCARDED));
    assert!(!requests.contains(texts::TERMS_EXIT_QUESTION));
    assert_eq!(draft_count(&app, "Inline exit").await, 0);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn summary_cancel_reports_that_no_draft_was_saved() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_periods_step(&app, &telegram, "Summary cancel").await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_CANCEL)
        .await;
    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .any(|message| message["text"] == texts::DRAFT_DISCARDED
                && message["reply_markup"]["keyboard"].is_array())
    );
    assert_eq!(draft_count(&app, "Summary cancel").await, 0);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_penalty_restored_over_a_deadline_it_cannot_follow_is_refused_and_the_bot_survives() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let id = assignment.id().as_uuid();
    let original = stored_assignment(&app, id).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let strict = cb(callbacks::LATE_POLICY_PREFIX, "none");
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_POLICY)
        .await;
    telegram.click_button(app.teacher(), &strict).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await;
    telegram.message(app.teacher(), &format!("30.12.{LATEST_TERM_YEAR}"));
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_POLICY)
        .await;

    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;

    let policy_step = telegram.button(app.teacher(), &strict).await;
    let refusal = telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| message["message_id"] == policy_step.message_id)
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(refusal.contains(texts::HORIZON_OUT_OF_RANGE), "{refusal}");
    telegram.click_button(app.teacher(), &strict).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_CANCEL)
        .await;
    let stored = stored_assignment(&app, id).await;
    assert_eq!(stored.deadline_at(), original.deadline_at());
    assert_eq!(stored.late_policy(), original.late_policy());
    stop_bot(cancellation, &mut task).await;
}

async fn open_edit(app: &TestApp, telegram: &TelegramScenario, id: uuid::Uuid, prefix: &str) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MANAGE_PREFIX, id))
        .await;
    telegram.flush(app.teacher()).await;
    telegram.click_button(app.teacher(), &cb(prefix, id)).await;
}

#[tokio::test]
async fn changing_the_due_date_in_telegram_shifts_the_final_closing() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let id = assignment.id().as_uuid();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(3));
    telegram.message(app.teacher(), &deadline);
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let saved = stored_assignment(&app, id).await;
    assert_eq!(saved.deadline_at(), expected_deadline(&deadline));
    assert_eq!(
        saved.closes_at().timestamp_micros(),
        (expected_deadline(&deadline) + chrono::Duration::weeks(4)).timestamp_micros()
    );
}

#[tokio::test]
async fn cancelling_a_terms_edit_preserves_all_conditions() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let id = assignment.id().as_uuid();
    let before = stored_assignment(&app, id).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await;
    telegram.message(
        app.teacher(),
        &deadline_input(Utc::now() + chrono::Duration::days(3)),
    );
    telegram
        .click_button(app.teacher(), callbacks::TERMS_PERIODS)
        .await;
    telegram.message(app.teacher(), "5");
    telegram.flush(app.teacher()).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_CANCEL)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(stored_assignment(&app, id).await, before);
    assert!(
        sent_callbacks(&telegram.server)
            .await
            .contains(&cb(callbacks::MANAGE_PREFIX, id))
    );
}

#[tokio::test]
async fn a_penalty_edit_does_not_overwrite_a_concurrent_scale_change() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, Some((7, 1, 4))).await;
    let id = assignment.id().as_uuid();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_POLICY)
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_POLICY_PREFIX, "none"))
        .await;
    let save = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    app.change_grading(id, app.teacher(), Some(10)).await.ok();
    telegram.click(app.teacher(), &save);
    telegram.flush(app.teacher()).await;
    telegram.click(app.teacher(), &save);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(
        stored_assignment(&app, id).await.grading().max_score(),
        Some(10)
    );
    let requests = recorded_requests(&telegram.server).await;
    assert!(requests.contains(texts::TERMS_CONFLICT), "{requests}");
    assert!(requests.contains(texts::STALE_BUTTON), "{requests}");
    let card_button = cb(callbacks::DEADLINE_PREFIX, id);
    assert_eq!(
        sent_callbacks(&telegram.server)
            .await
            .iter()
            .filter(|data| **data == card_button)
            .count(),
        2,
        "the card is shown again after the lost race"
    );
}

#[tokio::test]
async fn an_old_confirmation_cannot_save_the_new_date_dialogue() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(
        &app,
        &telegram,
        app.assignment(),
        callbacks::DEADLINE_PREFIX,
    )
    .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await;
    telegram.message(
        app.teacher(),
        &deadline_input(Utc::now() + chrono::Duration::days(10)),
    );
    let old = telegram.button(app.teacher(), callbacks::TERMS_SAVE).await;
    telegram.message(app.teacher(), "/cancel");
    telegram.flush(app.teacher()).await;
    open_edit(
        &app,
        &telegram,
        app.assignment(),
        callbacks::DEADLINE_PREFIX,
    )
    .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_DEADLINE)
        .await;
    let deadline = deadline_input(Utc::now() + chrono::Duration::days(20));
    telegram.message(app.teacher(), &deadline);
    let new = telegram
        .button_after(app.teacher(), callbacks::TERMS_SAVE, old.message_id)
        .await;
    let before = stored_assignment(&app, app.assignment()).await;
    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    assert_eq!(stored_assignment(&app, app.assignment()).await, before);
    telegram.click(app.teacher(), &new);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(
        stored_assignment(&app, app.assignment())
            .await
            .deadline_at(),
        expected_deadline(&deadline)
    );
}

#[tokio::test]
async fn a_teacher_can_enable_penalties_from_the_assignment_card() {
    let app = TestApp::start().await;
    let assignment = app.add_scored_assignment(5).await;
    let id = assignment.id().as_uuid();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_edit(&app, &telegram, id, callbacks::DEADLINE_PREFIX).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_POLICY)
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::LATE_POLICY_PREFIX, "work_score"),
        )
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::PENALTY_PREFIX, 1))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::LATE_PERIODS_PREFIX, 4))
        .await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;
    let saved = stored_assignment(&app, id).await;
    assert_eq!(saved.grading(), assignment.grading());
    assert_eq!(
        saved.closes_at(),
        saved.deadline_at() + chrono::Duration::weeks(4)
    );
    assert_eq!(saved.late_policy().unwrap().penalty_per_period(), 1);
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
        callback_update(10, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
        callback_update(11, app.teacher(), callbacks::FILE_FINISH, BOT_MESSAGE_ID),
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
        callback_update(8, app.teacher(), callbacks::TERMS_DEADLINE, BOT_MESSAGE_ID),
        message_update(9, app.teacher(), 2, &deadline_input),
        callback_update(10, app.teacher(), callbacks::TERMS_SAVE, BOT_MESSAGE_ID),
        callback_update(
            11,
            app.teacher(),
            &format!("{}{assignment}", callbacks::ATTEMPTS_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            12,
            app.teacher(),
            &format!("{}{student}", callbacks::STUDENT_PREFIX),
            BOT_MESSAGE_ID,
        ),
        message_update(13, app.teacher(), 3, "2"),
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

async fn open_groups_step(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), title);
    telegram.message(app.teacher(), "Send a pull request.");
}

fn group_toggle(group: &StudentGroup) -> String {
    cb(callbacks::ASSIGNMENT_GROUP_PREFIX, group.id().as_uuid())
}

fn group_button_text(
    telegram: &TelegramScenario,
    user: i64,
    message_id: i32,
    data: &str,
) -> String {
    telegram
        .messages(user)
        .into_iter()
        .find(|message| message["message_id"] == message_id)
        .unwrap()["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap())
        .find(|button| button["callback_data"] == data)
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn a_course_of_several_groups_issues_the_assignment_to_the_chosen_groups() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_groups_step(&app, &telegram, "Stream lab").await;

    let first = group_toggle(&app.fixture.group);
    let shown = telegram.button(app.teacher(), &first).await;
    let offered = [
        group_button_text(&telegram, app.teacher(), shown.message_id, &first),
        group_button_text(
            &telegram,
            app.teacher(),
            shown.message_id,
            &group_toggle(&second),
        ),
    ];
    let toggled = telegram.click_button(app.teacher(), &first).await;
    let unchecked = group_button_text(&telegram, app.teacher(), toggled.message_id, &first);
    telegram
        .click_button(app.teacher(), callbacks::ASSIGNMENT_GROUPS_DONE)
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADING_PREFIX, "passfail"))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    let summary = summary_text(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), callbacks::TERMS_SAVE)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        offered,
        [
            texts::group_choice_button(app.fixture.group.code().as_str(), true),
            texts::group_choice_button(second.code().as_str(), true),
        ]
    );
    assert_eq!(toggled.message_id, shown.message_id);
    assert_eq!(
        unchecked,
        texts::group_choice_button(app.fixture.group.code().as_str(), false)
    );
    assert!(
        summary.contains(&texts::groups_line(&[second.code().as_str()])),
        "{summary}"
    );
    let listed = app
        .course_assignments(app.course(), app.teacher())
        .await
        .ok();
    let created = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|assignment| assignment["title"] == "Stream lab")
        .unwrap_or_else(|| panic!("the draft was not saved: {listed:#}"));
    assert_eq!(created["student_group_ids"], json!([second.id().as_uuid()]));
}

#[tokio::test]
async fn continuing_without_a_group_keeps_the_group_step() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_groups_step(&app, &telegram, "Nobody").await;

    telegram
        .click_button(app.teacher(), &group_toggle(&app.fixture.group))
        .await;
    telegram
        .click_button(app.teacher(), &group_toggle(&second))
        .await;
    let refused = telegram
        .click_button(app.teacher(), callbacks::ASSIGNMENT_GROUPS_DONE)
        .await;
    let refusal = telegram
        .messages(app.teacher())
        .into_iter()
        .any(|message| message["text"] == texts::ASSIGNMENT_GROUPS_REQUIRED);
    telegram
        .click_button(app.teacher(), &group_toggle(&second))
        .await;
    let accepted = telegram
        .click_button(app.teacher(), callbacks::ASSIGNMENT_GROUPS_DONE)
        .await;
    telegram
        .button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(refusal, "the teacher is told that one group is needed");
    assert_eq!(refused.message_id, accepted.message_id);
}

#[tokio::test]
async fn a_course_of_one_group_skips_the_group_step() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_groups_step(&app, &telegram, "Single group").await;

    telegram
        .button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        telegram
            .messages(app.teacher())
            .iter()
            .all(|message| message["text"] != texts::ASSIGNMENT_GROUPS_QUESTION)
    );
}

#[tokio::test]
async fn a_stream_assignment_names_its_groups_on_the_card_and_in_the_attempts_list() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let third = app.add_group_to_course(&app.fixture.course).await;
    let course = app.reload_course(&app.fixture.course).await;
    let other = app.add_student_to_group("Other", &second).await;
    app.add_student_to_group("Skipped", &third).await;
    let assignment = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("Two groups")
        .groups([&app.fixture.group, &second])
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::MANAGE_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::ATTEMPTS_PREFIX, assignment.id().as_uuid()),
        )
        .await;
    let labelled = telegram
        .button_with_text(
            app.teacher(),
            &texts::student_button(&texts::group_student(
                second.code().as_str(),
                other.display_name().as_str(),
            )),
        )
        .await;
    stop_bot(cancellation, &mut task).await;

    let sent: Vec<String> = telegram
        .messages(app.teacher())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    let mut codes = vec![app.fixture.group.code().as_str(), second.code().as_str()];
    codes.sort_unstable();
    let groups = texts::groups_line(&codes);
    assert!(
        sent.iter()
            .any(|text| text.starts_with("Two groups") && text.contains(&groups)),
        "{sent:#?}"
    );
    assert!(labelled.data().ends_with(&other.id().as_uuid().to_string()));
    assert!(
        sent.iter().all(|text| !text.contains("Skipped")),
        "students of a group the assignment is not issued to are not listed: {sent:#?}"
    );
}

async fn open_attempts(app: &TestApp, telegram: &TelegramScenario, assignment: uuid::Uuid) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MANAGE_PREFIX, assignment))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::ATTEMPTS_PREFIX, assignment))
        .await;
}

fn has_live_buttons(telegram: &TelegramScenario, user: i64, message_id: i32) -> bool {
    telegram
        .messages(user)
        .into_iter()
        .find(|message| message["message_id"] == message_id)
        .expect("the question was sent")
        .get("reply_markup")
        .is_some()
}

#[tokio::test]
async fn an_assignment_without_students_does_not_ask_to_choose_one() {
    let app = TestApp::start().await;
    let empty = app.add_group_to_course(&app.fixture.course).await;
    let course = app.reload_course(&app.fixture.course).await;
    let assignment = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("Nobody yet")
        .groups([&empty])
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_attempts(&app, &telegram, assignment.id().as_uuid()).await;
    stop_bot(cancellation, &mut task).await;

    let sent: Vec<_> = telegram
        .messages(app.teacher())
        .into_iter()
        .map(|message| message["text"].clone())
        .collect();
    assert!(sent.contains(&json!(texts::NO_STUDENTS)), "{sent:#?}");
    assert!(
        !sent.contains(&json!(texts::CHOOSE_STUDENT_PROMPT)),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn a_chosen_attempt_limit_leaves_no_live_buttons() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), "Chosen limit");
    telegram.message(app.teacher(), "Send a pull request.");

    let attempts = telegram
        .click_button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram
        .button(app.teacher(), &cb(callbacks::GRADING_PREFIX, 5))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(!has_live_buttons(
        &telegram,
        app.teacher(),
        attempts.message_id
    ));
}

#[tokio::test]
async fn granted_attempts_leave_no_live_amount_buttons() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_attempts(&app, &telegram, app.assignment()).await;
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
        .click_button(app.teacher(), &cb(callbacks::EXTRA_ATTEMPTS_PREFIX, 1))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(!has_live_buttons(
        &telegram,
        app.teacher(),
        amount.message_id
    ));
}

#[tokio::test]
async fn typed_attempts_leave_no_live_amount_buttons() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_attempts(&app, &telegram, app.assignment()).await;
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

    telegram.message(app.teacher(), "1");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(!has_live_buttons(
        &telegram,
        app.teacher(),
        amount.message_id
    ));
}

#[tokio::test]
async fn a_negative_number_on_a_terms_step_is_answered_with_the_range() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_penalty_step(&app, &telegram, "Negative penalty").await;

    telegram.message(app.teacher(), "-1");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let requests = recorded_requests(&telegram.server).await;
    assert!(requests.contains(&texts::number_prompt(MAX_PENALTY_PER_PERIOD)));
}
