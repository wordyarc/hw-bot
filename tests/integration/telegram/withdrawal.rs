use chrono::{Duration, Utc};
use hw_bot::app::{ErrorCode, WithdrawSubmission};
use hw_bot::bot::{app_error_message, callbacks, texts};
use hw_bot::db;
use hw_bot::domain::{Submission, SubmissionId, SubmissionStatus};
use uuid::Uuid;

use super::flows::{open_assignment, open_submission_prompt};
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{Button, TelegramScenario, callbacks_of};
use crate::harness::{TestApp, insert_assignment, submission_id, wall_clock_assignment};
use hw_bot::test_support::submission as submission_of;

async fn stored(app: &TestApp, submission: Uuid) -> Submission {
    db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap()
}

async fn start_button(telegram: &TelegramScenario, user: i64, submission: Uuid) -> Button {
    telegram
        .button(user, &cb(callbacks::WITHDRAW_PREFIX, submission))
        .await
}

async fn confirm_button(telegram: &TelegramScenario, user: i64, after: i32) -> Button {
    let pressed = telegram
        .button_matching(user, |data| {
            data.starts_with(callbacks::WITHDRAW_CONFIRM_PREFIX)
        })
        .await;
    assert!(pressed.message_id > after, "a new confirmation is sent");
    pressed
}

fn offers_submission(telegram: &TelegramScenario, user: i64, assignment: Uuid) -> bool {
    let submit = cb(callbacks::SUBMIT_PREFIX, assignment);
    telegram
        .messages(user)
        .iter()
        .rev()
        .find(|message| !callbacks_of(message).is_empty())
        .is_some_and(|message| callbacks_of(message).contains(&submit))
}

#[tokio::test]
async fn a_work_withdrawn_from_the_hint_can_be_submitted_again() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let student = app.student();
    open_submission_prompt(&app, &telegram).await;
    telegram.message(student, "pull/wrong");
    let hint = telegram
        .button_matching(student, |data| data.starts_with(callbacks::WITHDRAW_PREFIX))
        .await;
    telegram.click(student, &hint);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, hint.message_id).await;

    telegram.click(student, &confirm);
    telegram.flush(student).await;
    stop_bot(cancellation, &mut task).await;

    let latest = db::submissions::latest_for_student(
        &app.pool,
        app.fixture.assignment.id(),
        app.fixture.student.id(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(latest.status(), SubmissionStatus::Withdrawn);
    assert!(
        telegram
            .texts(student)
            .contains(&texts::WITHDRAWN.to_owned())
    );
    assert!(offers_submission(&telegram, student, app.assignment()));
}

#[tokio::test]
async fn a_confirmation_outdated_by_the_closing_withdraws_nothing_and_asks_again() {
    let app = TestApp::start().await;
    let assignment = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
        .deadline_in(Duration::hours(3))
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;
    let submission = submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    );
    let telegram = TelegramScenario::new().await;
    let clock = TestClock::default();
    let (cancellation, mut task) = start_bot_with_clock(&app, &telegram.server, clock.clock());
    let student = app.student();
    open_assignment(&app, &telegram, assignment.id().as_uuid()).await;
    let start = start_button(&telegram, student, submission).await;
    telegram.click(student, &start);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, start.message_id).await;
    clock.move_to(assignment.deadline_at() + Duration::minutes(1));

    telegram.click(student, &confirm);
    telegram.flush(student).await;
    let reconfirm = confirm_button(&telegram, student, confirm.message_id).await;
    stop_bot(cancellation, &mut task).await;

    assert!(stored(&app, submission).await.is_pending_review());
    let refusal = texts::resubmission_refused(app_error_message(ErrorCode::DeadlineExceeded));
    let asked_again = telegram.message_by_id(student, reconfirm.message_id);
    assert!(
        asked_again["text"]
            .as_str()
            .is_some_and(|text| text.contains(&refusal)),
        "{asked_again:#}"
    );
}

#[tokio::test]
async fn pressing_the_confirmation_twice_withdraws_once() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let student = app.student();
    open_assignment(&app, &telegram, app.assignment()).await;
    let start = start_button(&telegram, student, submission).await;
    telegram.click(student, &start);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, start.message_id).await;

    telegram.click(student, &confirm);
    telegram.click(student, &confirm);
    telegram.flush(student).await;
    stop_bot(cancellation, &mut task).await;

    let sent = telegram.texts(student);
    let withdrawn = sent
        .iter()
        .filter(|text| text.as_str() == texts::WITHDRAWN)
        .count();
    assert_eq!(withdrawn, 1, "{sent:#?}");
    assert!(
        sent.contains(&app_error_message(ErrorCode::SubmissionAlreadyWithdrawn).to_owned()),
        "{sent:#?}"
    );
    assert_eq!(
        stored(&app, submission).await.status(),
        SubmissionStatus::Withdrawn
    );
}

#[tokio::test]
async fn an_old_withdraw_button_leaves_the_new_work_pending() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let student = app.student();
    open_assignment(&app, &telegram, app.assignment()).await;
    let start = start_button(&telegram, student, first).await;
    telegram.click(student, &start);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, start.message_id).await;
    telegram.click(student, &confirm);
    telegram.flush(student).await;
    let second = app.queued_submission(app.student(), "pull/2").await;

    telegram.click(student, &start);
    telegram.flush(student).await;
    stop_bot(cancellation, &mut task).await;

    assert!(stored(&app, second).await.is_pending_review());
    assert!(
        telegram
            .texts(student)
            .contains(&app_error_message(ErrorCode::SubmissionAlreadyWithdrawn).to_owned())
    );
}

#[tokio::test]
async fn a_withdrawal_during_the_review_comment_closes_the_review() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.open_review_queue(app.teacher()).await;
    telegram
        .click_button(
            app.teacher(),
            &cb(callbacks::REVIEW_REJECT_PREFIX, submission),
        )
        .await;
    app.application
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: app.student_actor(),
            withdrawn_at: Utc::now(),
        })
        .await
        .unwrap();

    telegram.message(app.teacher(), "Поздний комментарий");
    telegram.flush(app.teacher()).await;
    stop_bot(cancellation, &mut task).await;

    let sent = telegram.texts(app.teacher());
    assert!(
        sent.contains(&app_error_message(ErrorCode::SubmissionAlreadyWithdrawn).to_owned()),
        "{sent:#?}"
    );
    let stored = stored(&app, submission).await;
    assert_eq!(stored.status(), SubmissionStatus::Withdrawn);
    assert!(stored.review_comment().is_none());
}

#[tokio::test]
async fn a_confirmation_shown_after_the_closing_withdraws_the_work() {
    let app = TestApp::start().await;
    let assignment = app.add_overdue_assignment(1, None).await;
    let pending = submission_of(&assignment, &app.fixture.student)
        .submitted_at(assignment.deadline_at() - Duration::hours(1))
        .build();
    db::submissions::insert(&app.pool, &pending).await.unwrap();
    let submission = pending.id().as_uuid();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let student = app.student();
    open_assignment(&app, &telegram, assignment.id().as_uuid()).await;
    let start = start_button(&telegram, student, submission).await;
    telegram.click(student, &start);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, start.message_id).await;

    telegram.click(student, &confirm);
    telegram.flush(student).await;
    stop_bot(cancellation, &mut task).await;

    assert_eq!(
        stored(&app, submission).await.status(),
        SubmissionStatus::Withdrawn
    );
}

#[tokio::test]
async fn a_confirmation_outdated_by_an_archived_assignment_withdraws_nothing() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let student = app.student();
    open_assignment(&app, &telegram, app.assignment()).await;
    let start = start_button(&telegram, student, submission).await;
    telegram.click(student, &start);
    telegram.flush(student).await;
    let confirm = confirm_button(&telegram, student, start.message_id).await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    telegram.click(student, &confirm);
    telegram.flush(student).await;
    let reconfirm = confirm_button(&telegram, student, confirm.message_id).await;
    stop_bot(cancellation, &mut task).await;

    assert!(stored(&app, submission).await.is_pending_review());
    let refusal = texts::resubmission_refused(app_error_message(ErrorCode::AssignmentUnavailable));
    let asked_again = telegram.message_by_id(student, reconfirm.message_id);
    assert!(
        asked_again["text"]
            .as_str()
            .is_some_and(|text| text.contains(&refusal)),
        "{asked_again:#}"
    );
}
