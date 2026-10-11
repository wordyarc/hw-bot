use std::time::Duration;

use hw_bot::bot::{callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, body_partial_json, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;

#[tokio::test]
async fn help_command_resolves_the_registered_role_and_sends_its_commands() {
    let app = TestApp::start().await;
    let telegram = telegram_server(vec![message_update(1, app.teacher(), 1, "/help")]).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    wait_for_sent_message_count(&telegram, 1).await;
    stop_bot(cancellation, &mut task).await;
    let requests = telegram.received_requests().await.unwrap();
    let sent = requests
        .iter()
        .find(|request| request.url.path() == "/bottest-token/SendMessage")
        .expect("help was sent");
    let body: serde_json::Value = sent.body_json().unwrap();
    let text = body["text"].as_str().unwrap();
    assert_eq!(body["chat_id"], app.teacher());
    assert!(text.contains("/review"), "{text}");
    assert!(!text.contains("/tasks"), "{text}");
    assert!(
        !text.contains("/start"),
        "help must not silently fall back to guest: {text}"
    );
}

#[tokio::test]
async fn the_bottom_menu_opens_a_screen_even_in_the_middle_of_a_dialogue() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("{}{}", callbacks::SUBMIT_PREFIX, app.assignment()),
            BOT_MESSAGE_ID,
        ),
        message_update(3, app.student(), 2, texts::BUTTON_QUEUE),
    ];
    let telegram = telegram_server(updates).await;
    let queue = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "chat_id": app.student(),
            "text": texts::STUDENT_QUEUE_EMPTY
        })))
        .and(|request: &wiremock::Request| {
            has_callback_button(request, callbacks::MENU_ASSIGNMENTS)
        })
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), queue.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the queue screen did not open; requests:\n{requests}"));

    let submissions: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(
        submissions, 0,
        "a menu button is not the text of a submission"
    );
}

#[tokio::test]
async fn replayable_inline_navigation_cancels_the_previous_input_flow() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.student(), 1, "/menu"),
        callback_update(
            2,
            app.student(),
            &format!("{}{}", callbacks::SUBMIT_PREFIX, app.assignment()),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            3,
            app.student(),
            callbacks::MENU_ASSIGNMENTS,
            BOT_MESSAGE_ID - 5,
        ),
        message_update(4, app.student(), 2, "https://example.test/pull/1"),
    ];
    let telegram = telegram_server(updates).await;
    let idle = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": texts::USE_THE_MENU
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), idle.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the old flow remained active; requests:\n{requests}"));

    let submissions: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(submissions, 0, "navigation must cancel submission input");
}

#[tokio::test]
async fn the_first_start_in_a_fresh_chat_answers_with_the_menu() {
    let app = TestApp::start().await;
    let updates = vec![message_update(1, app.student(), 1, "/start")];
    let telegram = telegram_server(updates).await;
    let greeting = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_partial_json(json!({
            "chat_id": app.student(),
            "text": texts::home_greeting("Student")
        })))
        .and(|request: &wiremock::Request| has_reply_button(request, texts::BUTTON_ASSIGNMENTS))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), greeting.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("/start did not answer; requests:\n{requests}"));
}

#[tokio::test]
async fn navigation_from_an_older_message_still_opens_the_screen() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let telegram = crate::harness::telegram_scenario::TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/courses");
    let course = telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    let manage = telegram
        .click_button(app.teacher(), &cb(callbacks::MANAGE_PREFIX, assignment))
        .await;
    let attempts = telegram
        .button(app.teacher(), &cb(callbacks::ATTEMPTS_PREFIX, assignment))
        .await;
    let files = telegram
        .button(app.teacher(), &cb(callbacks::FILES_PREFIX, assignment))
        .await;
    telegram.message(app.teacher(), "/menu");
    telegram.flush(app.teacher()).await;

    for button in [&course, &manage, &attempts] {
        telegram.click(app.teacher(), button);
        telegram.flush(app.teacher()).await;
    }
    telegram
        .button(
            app.teacher(),
            &cb(
                callbacks::STUDENT_PREFIX,
                app.fixture.student.id().as_uuid(),
            ),
        )
        .await;
    telegram.click(app.teacher(), &files);
    telegram.flush(app.teacher()).await;
    telegram.button(app.teacher(), callbacks::FILE_ADD).await;
    stop_bot(cancellation, &mut task).await;
    assert!(
        !telegram
            .messages(app.teacher())
            .iter()
            .any(|message| message["text"] == texts::STALE_BUTTON)
    );
}

#[tokio::test]
async fn a_course_of_several_groups_is_listed_by_its_subject_and_lists_the_groups_inside() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/courses");
    let course = telegram
        .button_with_text(
            app.teacher(),
            &texts::course_groups_button(app.fixture.subject.name().as_str(), 2),
        )
        .await;
    telegram.click(app.teacher(), &course);
    telegram
        .button(app.teacher(), &cb(callbacks::CREATE_PREFIX, app.course()))
        .await;
    stop_bot(cancellation, &mut task).await;

    let mut codes = vec![app.fixture.group.code().as_str(), second.code().as_str()];
    codes.sort_unstable();
    let header = texts::course_header(app.fixture.subject.name().as_str(), &codes);
    let sent: Vec<String> = telegram
        .messages(app.teacher())
        .iter()
        .filter_map(|message| message["text"].as_str().map(str::to_owned))
        .collect();
    assert!(
        sent.iter().any(|text| text.starts_with(&header)),
        "{sent:#?}"
    );
}
