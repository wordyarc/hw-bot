use hw_bot::bot::{callbacks, texts};
use hw_bot::test_support::ASSIGNMENT_BODY;
use uuid::Uuid;

use crate::harness::TestApp;
use crate::harness::telegram::cb;
use crate::harness::telegram_scenario::{Button, TelegramScenario};

pub(super) async fn open_course(app: &TestApp, telegram: &TelegramScenario) -> Button {
    open_course_of(app, telegram, app.course()).await
}

pub(super) async fn open_course_of(
    app: &TestApp,
    telegram: &TelegramScenario,
    course: Uuid,
) -> Button {
    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, course))
        .await
}

pub(super) async fn manage_assignment(
    app: &TestApp,
    telegram: &TelegramScenario,
    assignment: Uuid,
) -> Button {
    open_course(app, telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MANAGE_PREFIX, assignment))
        .await
}

pub(super) async fn open_student_grades(
    app: &TestApp,
    telegram: &TelegramScenario,
    course: Uuid,
    student: Uuid,
) -> Button {
    open_course_of(app, telegram, course).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADES_PREFIX, course))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::STUDENT_PREFIX, student))
        .await
}

pub(super) async fn open_fixture_student_grades(
    app: &TestApp,
    telegram: &TelegramScenario,
) -> Button {
    open_student_grades(
        app,
        telegram,
        app.course(),
        app.fixture.student.id().as_uuid(),
    )
    .await
}

pub(super) fn student_card_header(name: &str) -> String {
    texts::grades_header(name, 0, 0, 0, 0)
        .lines()
        .next()
        .expect("the card starts with the student")
        .to_owned()
}

pub(super) async fn open_creation(app: &TestApp, telegram: &TelegramScenario, title: &str) {
    open_course(app, telegram).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    telegram.message(app.teacher(), title);
    telegram.message(app.teacher(), ASSIGNMENT_BODY);
}

pub(super) async fn open_scored_creation(
    app: &TestApp,
    telegram: &TelegramScenario,
    title: &str,
    max_score: u16,
) {
    open_creation(app, telegram, title).await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::MAX_ATTEMPTS_PREFIX, 2))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::GRADING_PREFIX, max_score))
        .await;
}

pub(super) async fn open_assignment(
    app: &TestApp,
    telegram: &TelegramScenario,
    assignment: Uuid,
) -> Button {
    telegram.message(app.student(), "/tasks");
    telegram
        .click_button(app.student(), &cb(callbacks::ASSIGNMENT_PREFIX, assignment))
        .await
}

pub(super) async fn open_submission_prompt(app: &TestApp, telegram: &TelegramScenario) -> Button {
    open_assignment(app, telegram, app.assignment()).await;
    telegram
        .click_button(
            app.student(),
            &cb(callbacks::SUBMIT_PREFIX, app.assignment()),
        )
        .await
}
