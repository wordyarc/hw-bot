use chrono::{Duration, Utc};
use hw_bot::app::{ChangeTerms, ConfigureMakeup, ErrorCode, MakeupSettings};
use hw_bot::bot::{app_error_message, callbacks, texts};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, AssignmentId, CourseMakeup, Grading, MakeupTerms, Submission, SubmissionId,
    SubmissionStatus, User,
};
use serde_json::Value;
use uuid::Uuid;

use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;
use crate::harness::{TestApp, submission_id};

fn ten() -> Grading {
    Grading::scored(10).unwrap()
}

fn credit(value: u16) -> MakeupTerms {
    MakeupTerms::scored(value, ten()).unwrap()
}

fn answer(value: impl std::fmt::Display) -> String {
    cb(callbacks::MAKEUP_ANSWER_PREFIX, value)
}

async fn open_makeup(app: &TestApp, attempts: u16) -> CourseMakeup {
    app.open_makeup(
        &app.fixture.course,
        attempts,
        Utc::now() - Duration::minutes(1),
    )
    .await
}

async fn settings(app: &TestApp) -> Option<CourseMakeup> {
    app.application
        .courses
        .active_makeup(app.teacher_actor(), app.fixture.course.id())
        .await
        .unwrap()
}

async fn terms_of(app: &TestApp, assignment: AssignmentId) -> Option<MakeupTerms> {
    db::assignments::find_by_id(&app.pool, assignment)
        .await
        .unwrap()
        .unwrap()
        .makeup()
}

async fn pool_balance(app: &TestApp) -> (u16, u16) {
    let pool = app
        .application
        .courses
        .student_makeup(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
        )
        .await
        .unwrap()
        .unwrap();
    (pool.balance.remaining(), pool.balance.allowed())
}

async fn stored(app: &TestApp, submission: Uuid) -> Submission {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap()
}

fn grade_text(submission: &Submission) -> String {
    texts::grade_line(submission.kind(), submission.grade()).expect("a makeup is always marked")
}

fn texts_sent(telegram: &TelegramScenario, user: i64) -> Vec<String> {
    telegram
        .messages(user)
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect()
}

fn sent_message(telegram: &TelegramScenario, user: i64, message_id: i32) -> Value {
    telegram
        .messages(user)
        .into_iter()
        .find(|message| message["message_id"] == message_id)
        .expect("the message was sent")
}

fn callbacks_of(message: &Value) -> Vec<String> {
    message["reply_markup"]["inline_keyboard"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row.as_array().unwrap().clone())
        .filter_map(|button| button["callback_data"].as_str().map(str::to_owned))
        .collect()
}

async fn makeup_of(app: &TestApp, assignment: &Assignment, student: &User) -> Uuid {
    submission_id(
        &app.submit(
            assignment.id().as_uuid(),
            student.telegram_user_id().as_i64(),
            "pull/makeup",
        )
        .await
        .created(),
    )
}

async fn open_form(app: &TestApp, telegram: &TelegramScenario) {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAKEUP_PREFIX, app.course()))
        .await;
}

#[tokio::test]
async fn enabling_with_a_date_a_pool_and_two_scales_writes_only_on_save() {
    let app = TestApp::start().await;
    let first = app.add_scored_assignment(10).await;
    let second = app.add_scored_assignment(10).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();

    open_form(&app, &telegram).await;
    telegram
        .click_button(teacher, &cb(callbacks::DEADLINE_MONTHS_PREFIX, 1))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::MAKEUP_ATTEMPTS_PREFIX, 3))
        .await;
    telegram.click_button(teacher, &answer(6)).await;
    telegram.click_button(teacher, &answer("pass")).await;
    telegram.button(teacher, callbacks::MAKEUP_SAVE).await;
    assert_eq!(settings(&app).await, None);
    assert_eq!(terms_of(&app, first.id()).await, None);
    telegram.click_button(teacher, callbacks::MAKEUP_SAVE).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(settings(&app).await.map(CourseMakeup::attempts), Some(3));
    assert_eq!(terms_of(&app, first.id()).await, Some(credit(6)));
    assert_eq!(terms_of(&app, second.id()).await, Some(credit(6)));
    assert_eq!(
        terms_of(&app, app.fixture.assignment.id()).await,
        Some(MakeupTerms::PassFail)
    );
    let sent = texts_sent(&telegram, teacher);
    assert!(sent.contains(&texts::MAKEUP_SAVED.to_owned()), "{sent:#?}");
    assert!(
        sent.iter()
            .any(|text| text.starts_with(texts::MAKEUP_SUMMARY_HEADER)
                && text.contains(&texts::makeup_attempts_line(3))),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn a_menu_item_during_the_form_writes_nothing_and_says_so() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_form(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_MONTHS_PREFIX, 1))
        .await;
    telegram
        .button(app.teacher(), &cb(callbacks::MAKEUP_ATTEMPTS_PREFIX, 3))
        .await;

    telegram.message(app.teacher(), texts::BUTTON_COURSES);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.contains(&texts::MAKEUP_DISCARDED.to_owned()),
        "{sent:#?}"
    );
    assert!(sent.contains(&texts::CHOOSE_COURSE.to_owned()), "{sent:#?}");
    assert_eq!(settings(&app).await, None);
}

#[tokio::test]
async fn a_save_opened_before_a_colleagues_save_keeps_the_colleagues_settings() {
    let app = TestApp::start().await;
    let colleague = app.add_course_teacher("Colleague").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_form(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_MONTHS_PREFIX, 1))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAKEUP_ATTEMPTS_PREFIX, 3))
        .await;
    telegram.click_button(app.teacher(), &answer("pass")).await;
    app.application
        .courses
        .configure_makeup(ConfigureMakeup {
            course_id: app.fixture.course.id(),
            actor: hw_bot::app::Actor::new(colleague.id()),
            expected: None,
            settings: Some(MakeupSettings {
                until: Utc::now() + Duration::days(20),
                attempts: 5,
            }),
            decisions: Vec::new(),
            now: Utc::now(),
        })
        .await
        .unwrap();

    telegram
        .click_button(app.teacher(), callbacks::MAKEUP_SAVE)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(settings(&app).await.map(CourseMakeup::attempts), Some(5));
    assert_eq!(terms_of(&app, app.fixture.assignment.id()).await, None);
    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::MAKEUP_CONFLICT.to_owned()));
}

#[tokio::test]
async fn a_prolongation_saved_after_the_makeup_ended_writes_nothing_and_announces_a_new_pool() {
    let app = TestApp::start().await;
    let opened = app
        .open_makeup(
            &app.fixture.course,
            3,
            Utc::now() - Duration::days(30) + Duration::seconds(10),
        )
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_form(&app, &telegram).await;
    telegram.click_button(app.teacher(), &answer("pass")).await;
    telegram
        .click_button(app.teacher(), callbacks::MAKEUP_DATE)
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::DEADLINE_MONTHS_PREFIX, 1))
        .await;
    telegram.button(app.teacher(), callbacks::MAKEUP_SAVE).await;
    tokio::time::sleep((opened.until() - Utc::now()).to_std().unwrap_or_default()).await;

    telegram
        .click_button(app.teacher(), callbacks::MAKEUP_SAVE)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        db::makeup::find_course(&app.pool, app.fixture.course.id())
            .await
            .unwrap(),
        Some(opened)
    );
    assert!(texts_sent(&telegram, app.teacher()).iter().any(|text| {
        text.starts_with(texts::MAKEUP_ENDED_BEFORE_SAVE) && text.contains(texts::MAKEUP_NEW_POOL)
    }));
}

async fn create_through_the_wizard(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    let teacher = app.teacher();
    telegram.message(teacher, "/courses");
    telegram
        .click_button(teacher, &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(teacher, title);
    telegram.message(teacher, "Send a pull request.");
    telegram
        .click_button(teacher, &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::GRADING_PREFIX, 10))
        .await;
}

async fn finish_strict_creation(app: &TestApp, telegram: &TelegramScenario) {
    let teacher = app.teacher();
    telegram
        .click_button(teacher, &cb(callbacks::LATE_POLICY_PREFIX, "none"))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::DEADLINE_DAYS_PREFIX, 7))
        .await;
    telegram.click_button(teacher, callbacks::TERMS_SAVE).await;
}

async fn created(app: &TestApp, title: &str) -> AssignmentId {
    let created = wait_for_created_assignment(&app.pool, title)
        .await
        .expect("the draft is created");
    AssignmentId::from_uuid(created.id)
}

#[tokio::test]
async fn creating_on_a_course_with_makeup_saves_the_credit() {
    let app = TestApp::start().await;
    open_makeup(&app, 3).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    create_through_the_wizard(&app, &telegram, "Makeup draft").await;
    telegram.click_button(app.teacher(), &answer(6)).await;
    finish_strict_creation(&app, &telegram).await;
    let id = created(&app, "Makeup draft").await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(terms_of(&app, id).await, Some(credit(6)));
}

#[tokio::test]
async fn creating_on_a_course_without_makeup_asks_nothing_and_leaves_the_terms_undecided() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    create_through_the_wizard(&app, &telegram, "Plain draft").await;
    finish_strict_creation(&app, &telegram).await;
    let id = created(&app, "Plain draft").await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(terms_of(&app, id).await, None);
    assert!(
        sent_callbacks(&telegram.server)
            .await
            .iter()
            .all(|data| !data.starts_with(callbacks::MAKEUP_ANSWER_PREFIX))
    );
}

#[tokio::test]
async fn editing_terms_on_a_course_without_makeup_keeps_the_makeup_terms() {
    let app = TestApp::start().await;
    let work = app
        .add_makeup_assignment(
            "Credited",
            ten(),
            credit(6),
            Utc::now() + Duration::days(10),
        )
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();

    telegram.message(teacher, "/courses");
    telegram
        .click_button(teacher, &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::MANAGE_PREFIX, work.id().as_uuid()))
        .await;
    telegram
        .click_button(
            teacher,
            &cb(callbacks::DEADLINE_PREFIX, work.id().as_uuid()),
        )
        .await;
    telegram
        .click_button(teacher, callbacks::TERMS_DEADLINE)
        .await;
    telegram
        .click_button(teacher, &cb(callbacks::DEADLINE_DAYS_PREFIX, 14))
        .await;
    telegram.click_button(teacher, callbacks::TERMS_SAVE).await;
    stop_bot(cancellation, &mut task).await;

    let stored = db::assignments::find_by_id(&app.pool, work.id())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stored.deadline_at(), work.deadline_at());
    assert_eq!(stored.makeup(), Some(credit(6)));
    assert!(
        sent_callbacks(&telegram.server)
            .await
            .iter()
            .all(|data| data != callbacks::TERMS_MAKEUP)
    );
}

#[tokio::test]
async fn the_review_entry_offers_the_queue_and_the_makeups_when_makeups_wait() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    app.queued_submission(app.student(), "pull/regular").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_REVIEW);
    let makeups = telegram
        .button(app.teacher(), callbacks::NEXT_MAKEUP_REVIEW)
        .await;
    let queue = telegram.button(app.teacher(), callbacks::NEXT_REVIEW).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(makeups.message_id, queue.message_id);
    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::review_hub(1, 1)));
}

#[tokio::test]
async fn without_makeups_the_review_entry_opens_the_next_work() {
    let app = TestApp::start().await;
    let regular = app.queued_submission(app.student(), "pull/regular").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_REVIEW);
    telegram
        .button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, regular))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        texts_sent(&telegram, app.teacher())
            .iter()
            .all(|text| !text.starts_with("✅ Проверка работ"))
    );
}

#[tokio::test]
async fn the_home_screen_counts_makeups_apart_from_the_queue() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    for name in ["First", "Second", "Third", "Fourth", "Fifth"] {
        let student = app.add_student(name).await;
        makeup_of(&app, &closed, &student).await;
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
    assert!(home.contains(&texts::makeup_queue_line(5)), "{home}");
}

async fn open_next_makeup(app: &TestApp, telegram: &TelegramScenario) {
    telegram.message(app.teacher(), texts::BUTTON_REVIEW);
    telegram
        .click_button(app.teacher(), callbacks::NEXT_MAKEUP_REVIEW)
        .await;
}

#[tokio::test]
async fn crediting_a_makeup_saves_its_credit_after_the_comment() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_next_makeup(&app, &telegram).await;
    let accept = telegram
        .button_with_text(app.teacher(), &texts::accept_makeup_button(6))
        .await;
    telegram.click(app.teacher(), &accept);
    telegram.flush(app.teacher()).await;
    telegram.message(app.teacher(), "Досдано");
    let reviewed = wait_for_review_result(&app.pool, makeup).await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(reviewed, Some(("accepted".to_owned(), Some(6), Some(6))));
    let credited = grade_text(&stored(&app, makeup).await);
    assert!(credited.contains("6 из 10"), "{credited}");
    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter()
            .any(|text| text.starts_with(texts::REVIEW_SAVED) && text.contains(&credited)),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn changing_the_decision_brings_back_the_makeup_buttons() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_next_makeup(&app, &telegram).await;
    let first = telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, makeup))
        .await;

    telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_CHANGE_PREFIX, makeup))
        .await;
    let again = telegram
        .button_after(
            app.teacher(),
            &cb(callbacks::REVIEW_ACCEPT_PREFIX, makeup),
            first.message_id,
        )
        .await;
    stop_bot(cancellation, &mut task).await;

    let card = sent_message(&telegram, app.teacher(), again.message_id);
    let labels = card["reply_markup"]["inline_keyboard"].to_string();
    assert!(labels.contains(&texts::accept_makeup_button(6)), "{labels}");
}

#[tokio::test]
async fn withdrawn_terms_before_saving_leave_only_the_rejection() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_next_makeup(&app, &telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, makeup))
        .await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: closed.id(),
            actor: app.teacher_actor(),
            deadline_at: closed.deadline_at(),
            late_policy: None,
            makeup: Some(MakeupTerms::NotOffered),
            expected_revision: 0,
        })
        .await
        .unwrap();

    telegram.message(app.teacher(), "Досдано");
    let reject = cb(callbacks::REVIEW_REJECT_PREFIX, makeup);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        texts_sent(&telegram, app.teacher()).contains(&texts::REVIEW_MAKEUP_WITHDRAWN.to_owned())
    );
    let card = telegram
        .messages(app.teacher())
        .into_iter()
        .rev()
        .find(|message| callbacks_of(message).contains(&reject))
        .expect("the card is shown again");
    assert!(
        !callbacks_of(&card).contains(&cb(callbacks::REVIEW_ACCEPT_PREFIX, makeup)),
        "{card:#}"
    );
    assert!(stored(&app, makeup).await.is_pending_review());
}

#[tokio::test]
async fn the_next_makeup_button_opens_the_following_makeup() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    let second_student = app.add_student("Second").await;
    open_makeup(&app, 3).await;
    let first = makeup_of(&app, &closed, &app.fixture.student).await;
    let second = makeup_of(&app, &closed, &second_student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_next_makeup(&app, &telegram).await;
    let card = telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, first))
        .await;
    telegram.message(app.teacher(), "Досдано");
    wait_for_review_result(&app.pool, first)
        .await
        .expect("the first makeup is reviewed");

    let next = telegram
        .button_after(
            app.teacher(),
            callbacks::NEXT_MAKEUP_REVIEW,
            card.message_id,
        )
        .await;
    telegram.click(app.teacher(), &next);
    telegram
        .button(app.teacher(), &cb(callbacks::REVIEW_ACCEPT_PREFIX, second))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::makeups_left(1)));
}

async fn reject_the_only_makeup(app: &TestApp, telegram: &TelegramScenario, makeup: Uuid) {
    open_next_makeup(app, telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_REJECT_PREFIX, makeup))
        .await;
    telegram.message(app.teacher(), "Не то");
    wait_for_review_result(&app.pool, makeup)
        .await
        .expect("the makeup is rejected");
}

#[tokio::test]
async fn a_grant_after_a_rejection_gives_the_student_an_attempt() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 1).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    reject_the_only_makeup(&app, &telegram, makeup).await;

    let grant = telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;
    telegram.click(app.teacher(), &grant);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(pool_balance(&app).await, (1, 2));
    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter()
            .any(|text| text.contains(&texts::review_makeup_attempts(0, 1))),
        "{sent:#?}"
    );
    let granted = sent
        .iter()
        .position(|text| text == texts::MAKEUP_ATTEMPT_GRANTED)
        .unwrap_or_else(|| panic!("{sent:#?}"));
    let balance = sent
        .iter()
        .position(|text| text.contains(&texts::review_makeup_attempts(1, 2)))
        .unwrap_or_else(|| panic!("{sent:#?}"));
    assert!(granted < balance, "{sent:#?}");
}

#[tokio::test]
async fn a_grant_from_a_stale_message_adds_nothing() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 1).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    reject_the_only_makeup(&app, &telegram, makeup).await;
    let old = telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;
    telegram.click(app.teacher(), &old);
    telegram
        .button_after(app.teacher(), callbacks::MAKEUP_GRANT, old.message_id)
        .await;

    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(pool_balance(&app).await, (1, 2));
    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::STALE_BUTTON.to_owned()));
}

async fn open_card(app: &TestApp, telegram: &TelegramScenario, assignment: &Assignment) {
    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, assignment.id().as_uuid()),
        )
        .await;
}

#[tokio::test]
async fn a_closed_assignment_with_makeup_prompts_and_confirms_with_the_remaining_attempts() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 2).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_card(&app, &telegram, &closed).await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, closed.id().as_uuid()),
        )
        .await;
    telegram.message(app.student(), "pull/makeup");
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let terms = texts::makeup_credit_text(Some((6, 10)));
    let sent = texts_sent(&telegram, app.student());
    assert!(
        sent.iter()
            .any(|text| text.contains(&texts::card_makeup_attempts(2, 2))),
        "{sent:#?}"
    );
    assert!(
        sent.contains(&texts::input_prompt(&texts::makeup_prompt(&terms, 2))),
        "{sent:#?}"
    );
    assert!(sent.contains(&texts::makeup_queued(1)), "{sent:#?}");
}

#[tokio::test]
async fn an_empty_pool_leaves_the_card_without_a_submit_button() {
    let app = TestApp::start().await;
    let spent = app.add_closed_assignment("Spent", 10, 6).await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 1).await;
    makeup_of(&app, &spent, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_card(&app, &telegram, &closed).await;
    stop_bot(cancellation, &mut task).await;

    let submit = cb(callbacks::SUBMIT_PREFIX, closed.id().as_uuid());
    assert!(
        telegram
            .messages(app.student())
            .iter()
            .all(|message| !callbacks_of(message).contains(&submit))
    );
    assert!(
        texts_sent(&telegram, app.student())
            .iter()
            .any(|text| text.contains(texts::MAKEUP_ATTEMPTS_EXHAUSTED))
    );
}

#[tokio::test]
async fn an_old_makeup_button_names_the_spent_pool_instead_of_asking_for_work() {
    let app = TestApp::start().await;
    let spent = app.add_closed_assignment("Spent", 10, 6).await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 1).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_card(&app, &telegram, &closed).await;
    let submit = telegram
        .button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, closed.id().as_uuid()),
        )
        .await;
    makeup_of(&app, &spent, &app.fixture.student).await;
    telegram.click(app.student(), &submit);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.student());
    assert!(
        sent.contains(&app_error_message(ErrorCode::MakeupAttemptsExhausted).to_owned()),
        "{sent:#?}"
    );
    assert!(
        !sent
            .iter()
            .any(|text| text.contains(texts::SUBMISSION_PROMPT)),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn a_regular_prompt_answered_after_the_closing_shows_the_makeup_card_and_keeps_the_pool() {
    let app = TestApp::start().await;
    let work = app
        .add_makeup_assignment("Closing", ten(), credit(6), Utc::now() + Duration::hours(1))
        .await;
    open_makeup(&app, 2).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_card(&app, &telegram, &work).await;
    let submit = telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, work.id().as_uuid()),
        )
        .await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: work.id(),
            actor: app.teacher_actor(),
            deadline_at: Utc::now() - Duration::minutes(5),
            late_policy: None,
            makeup: work.makeup(),
            expected_revision: 0,
        })
        .await
        .unwrap();

    telegram.message(app.student(), "pull/late");
    let makeup = telegram
        .button_after(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, work.id().as_uuid()),
            submit.message_id,
        )
        .await;
    stop_bot(cancellation, &mut task).await;

    let card = telegram
        .messages(app.student())
        .into_iter()
        .find(|message| message["message_id"] == makeup.message_id)
        .unwrap();
    assert!(
        card["reply_markup"]["inline_keyboard"]
            .to_string()
            .contains(texts::BUTTON_SUBMIT_MAKEUP)
    );
    assert!(
        texts_sent(&telegram, app.student())
            .contains(&app_error_message(ErrorCode::DeadlineExceeded).to_owned())
    );
    let submitted: i64 =
        sqlx::query_scalar("select count(*) from submissions where assignment_id = $1")
            .bind(work.id().as_uuid())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(submitted, 0);
    assert_eq!(pool_balance(&app).await, (2, 2));
}

async fn open_student_grades(app: &TestApp, telegram: &TelegramScenario) {
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
}

async fn spend_the_whole_pool(app: &TestApp, attempts: u16) {
    open_makeup(app, attempts).await;
    for index in 0..attempts {
        let closed = app
            .add_closed_assignment(&format!("Closed {index}"), 10, 6)
            .await;
        makeup_of(app, &closed, &app.fixture.student).await;
    }
}

#[tokio::test]
async fn a_grant_from_the_gradebook_shows_the_card_again_with_the_new_balance() {
    let app = TestApp::start().await;
    spend_the_whole_pool(&app, 3).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_student_grades(&app, &telegram).await;
    let old = telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;

    telegram.click(app.teacher(), &old);
    telegram
        .button_after(app.teacher(), callbacks::MAKEUP_GRANT, old.message_id)
        .await;
    telegram.click(app.teacher(), &old);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(pool_balance(&app).await, (1, 4));
    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter()
            .any(|text| text.contains(&texts::gradebook_makeup_attempts(0, 3))),
        "{sent:#?}"
    );
    assert!(
        sent.iter()
            .any(|text| text.contains(&texts::gradebook_makeup_attempts(1, 4))),
        "{sent:#?}"
    );
    assert!(sent.contains(&texts::STALE_BUTTON.to_owned()), "{sent:#?}");
}

#[tokio::test]
async fn a_course_without_makeup_has_no_makeup_line_in_the_gradebook() {
    let app = TestApp::start().await;
    app.add_scored_assignment(10).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_student_grades(&app, &telegram).await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let header = format!("👤 {}", app.fixture.student.display_name().as_str());
    let card = texts_sent(&telegram, app.teacher())
        .into_iter()
        .find(|text| text.starts_with(&header))
        .expect("the student card is shown");
    assert!(!card.contains(texts::MAKEUP_MARK), "{card}");
    assert!(
        sent_callbacks(&telegram.server)
            .await
            .iter()
            .all(|data| data != callbacks::MAKEUP_GRANT)
    );
}

#[tokio::test]
async fn the_history_marks_a_credited_makeup() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    app.review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    open_card(&app, &telegram, &closed).await;
    let history = telegram
        .button_with_text(app.student(), texts::BUTTON_HISTORY)
        .await;
    telegram.click(app.student(), &history);
    telegram.flush(app.student()).await;
    stop_bot(cancellation, &mut task).await;

    let attempt = texts::attempt_line(
        1,
        texts::submission_status(SubmissionStatus::Accepted),
        Some(&grade_text(&stored(&app, makeup).await)),
    );
    assert!(texts_sent(&telegram, app.student()).contains(&attempt));
}

#[tokio::test]
async fn without_queued_work_the_review_entry_offers_only_the_makeups() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), texts::BUTTON_REVIEW);
    let makeups = telegram
        .button(app.teacher(), callbacks::NEXT_MAKEUP_REVIEW)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(texts_sent(&telegram, app.teacher()).contains(&texts::review_hub(0, 1)));
    let hub = sent_message(&telegram, app.teacher(), makeups.message_id);
    assert!(
        !callbacks_of(&hub).contains(&callbacks::NEXT_REVIEW.to_owned()),
        "{hub:#}"
    );
}

#[tokio::test]
async fn an_emptied_queue_names_the_makeups_still_waiting() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    let regular = app.queued_submission(app.student(), "pull/regular").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_REVIEW);
    let queue = telegram.button(app.teacher(), callbacks::NEXT_REVIEW).await;
    telegram.click(app.teacher(), &queue);
    telegram
        .click_button(app.teacher(), &cb(callbacks::REVIEW_REJECT_PREFIX, regular))
        .await;
    telegram.message(app.teacher(), "Не то");
    wait_for_review_result(&app.pool, regular)
        .await
        .expect("the regular work is reviewed");

    telegram.click(app.teacher(), &queue);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter()
            .any(|text| text.starts_with(texts::REVIEW_QUEUE_EMPTY)
                && text.contains(&texts::makeup_queue_line(1))),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn a_credit_lowered_after_the_card_was_shown_is_previewed_at_its_new_value() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_next_makeup(&app, &telegram).await;
    let accept = telegram
        .button_with_text(app.teacher(), &texts::accept_makeup_button(6))
        .await;
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: closed.id(),
            actor: app.teacher_actor(),
            deadline_at: closed.deadline_at(),
            late_policy: None,
            makeup: Some(credit(5)),
            expected_revision: 0,
        })
        .await
        .unwrap();

    telegram.click(app.teacher(), &accept);
    telegram.flush(app.teacher()).await;
    telegram.message(app.teacher(), "Досдано");
    let reviewed = wait_for_review_result(&app.pool, makeup).await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(reviewed, Some(("accepted".to_owned(), Some(5), Some(5))));
    let credited = grade_text(&stored(&app, makeup).await);
    let sent = texts_sent(&telegram, app.teacher());
    assert!(
        sent.iter()
            .any(|text| !text.starts_with(texts::REVIEW_SAVED) && text.contains(&credited)),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn rejecting_a_sandbox_makeup_offers_a_grant_without_the_makeup_queue() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_makeup_assignment_on(
            &sandbox,
            "Trial",
            ten(),
            credit(6),
            Utc::now() - Duration::days(1),
        )
        .await;
    app.open_makeup(&sandbox, 1, Utc::now() - Duration::minutes(1))
        .await;
    let trial_makeup = makeup_of(&app, &trial, &app.fixture.student).await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    open_makeup(&app, 3).await;
    makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), texts::BUTTON_SANDBOX);
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_OPEN_PREFIX, trial_makeup),
        )
        .await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_REJECT_PREFIX, trial_makeup),
        )
        .await;
    telegram.message(app.teacher(), "Учебная");
    wait_for_review_result(&app.pool, trial_makeup)
        .await
        .expect("the sandbox makeup is rejected");

    let grant = telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;
    stop_bot(cancellation, &mut task).await;

    let message = sent_message(&telegram, app.teacher(), grant.message_id);
    assert!(
        !callbacks_of(&message).contains(&callbacks::NEXT_MAKEUP_REVIEW.to_owned()),
        "{message:#}"
    );
    let followup = message["text"].as_str().unwrap();
    assert!(
        followup.contains(&texts::review_makeup_attempts(0, 1)),
        "{followup}"
    );
    assert!(!followup.contains(&texts::makeups_left(1)), "{followup}");
}

#[tokio::test]
async fn the_student_list_stays_usable_after_a_card_is_opened() {
    let app = TestApp::start().await;
    let second = app.add_student("Second").await;
    open_makeup(&app, 3).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    open_student_grades(&app, &telegram).await;
    telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;

    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::STUDENT_PREFIX, second.id().as_uuid()),
        )
        .await;
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = texts_sent(&telegram, app.teacher());
    assert!(!sent.contains(&texts::STALE_BUTTON.to_owned()), "{sent:#?}");
    let header = format!("👤 {}", second.display_name().as_str());
    assert!(
        sent.iter().any(|text| text.starts_with(&header)),
        "{sent:#?}"
    );
}

#[tokio::test]
async fn an_ended_makeup_is_shown_as_off_and_is_set_up_again_from_the_date() {
    let app = TestApp::start().await;
    app.open_makeup(&app.fixture.course, 3, Utc::now() - Duration::days(40))
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;

    let off = telegram
        .button_with_text(app.teacher(), texts::BUTTON_MAKEUP_OFF)
        .await;
    telegram.click(app.teacher(), &off);
    telegram
        .button(app.teacher(), &cb(callbacks::DEADLINE_MONTHS_PREFIX, 1))
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(
        texts_sent(&telegram, app.teacher())
            .iter()
            .any(|text| text.contains(texts::MAKEUP_DATE_QUESTION))
    );
}

#[tokio::test]
async fn a_step_opened_from_the_summary_returns_to_it_by_the_back_button() {
    let app = TestApp::start().await;
    open_makeup(&app, 3).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();

    open_form(&app, &telegram).await;
    telegram.click_button(teacher, &answer("pass")).await;
    telegram
        .click_button(teacher, callbacks::MAKEUP_ATTEMPTS)
        .await;
    telegram.click_button(teacher, callbacks::MAKEUP_BACK).await;
    telegram.click_button(teacher, callbacks::MAKEUP_SAVE).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(settings(&app).await.map(CourseMakeup::attempts), Some(3));
    assert!(texts_sent(&telegram, teacher).contains(&texts::MAKEUP_SAVED.to_owned()));
}

#[tokio::test]
async fn a_grant_after_the_makeup_was_turned_off_explains_it_and_drops_the_button() {
    let app = TestApp::start().await;
    let closed = app.add_closed_assignment("Closed", 10, 6).await;
    let opened = open_makeup(&app, 1).await;
    let makeup = makeup_of(&app, &closed, &app.fixture.student).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    reject_the_only_makeup(&app, &telegram, makeup).await;
    let grant = telegram
        .button(app.teacher(), callbacks::MAKEUP_GRANT)
        .await;
    app.application
        .courses
        .configure_makeup(ConfigureMakeup {
            course_id: app.fixture.course.id(),
            actor: app.teacher_actor(),
            expected: Some(opened),
            settings: None,
            decisions: Vec::new(),
            now: Utc::now(),
        })
        .await
        .unwrap();

    telegram.click(app.teacher(), &grant);
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let notice = telegram
        .messages(app.teacher())
        .into_iter()
        .find(|message| {
            message["text"]
                .as_str()
                .is_some_and(|text| text.starts_with(texts::MAKEUP_GRANT_UNAVAILABLE))
        })
        .expect("the refusal is explained");
    assert!(!callbacks_of(&notice).contains(&callbacks::MAKEUP_GRANT.to_owned()));
}
