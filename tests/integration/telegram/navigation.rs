use std::time::Duration;

use hw_bot::bot::{callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;

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
        message_update(3, app.student(), 2, "⏳ Очередь проверки"),
    ];
    let telegram = telegram_server(updates).await;
    let queue = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": texts::STUDENT_QUEUE_EMPTY,
            "reply_markup": {
                "inline_keyboard": [
                    [{ "text": "📚 Мои задания", "callback_data": callbacks::MENU_ASSIGNMENTS }],
                    [
                        { "text": "⏳ Очередь проверки", "callback_data": callbacks::MENU_QUEUE },
                        { "text": "🗄 Архив", "callback_data": callbacks::MENU_ARCHIVE }
                    ],
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
async fn stale_assignment_button_does_not_mutate_the_assignment() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
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
            BOT_MESSAGE_ID - 1,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": texts::STALE_BUTTON
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("stale callback was not rejected; requests:\n{requests}"));

    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(assignment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "published");
}

#[tokio::test]
async fn the_first_start_in_a_fresh_chat_answers_with_the_menu() {
    let app = TestApp::start().await;
    let updates = vec![message_update(1, app.student(), 1, "/start")];
    let telegram = telegram_server(updates).await;
    let greeting = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.student(),
            "text": "👋 Здравствуйте, Student!\nВыберите пункт меню ниже 👇",
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
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(2, app.teacher(), callbacks::MENU_COURSES, BOT_MESSAGE_ID),
        // Экран курса открыт с более нового сообщения, а жмём кнопки со старого.
        callback_update(
            3,
            app.teacher(),
            &format!("{}{}", callbacks::COURSE_PREFIX, app.course()),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            4,
            app.teacher(),
            &format!("{}{assignment}", callbacks::MANAGE_PREFIX),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            5,
            app.teacher(),
            &format!("{}{assignment}", callbacks::ATTEMPTS_PREFIX),
            BOT_MESSAGE_ID - 5,
        ),
        callback_update(
            6,
            app.teacher(),
            &format!("{}{assignment}", callbacks::FILES_PREFIX),
            BOT_MESSAGE_ID - 5,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": texts::STALE_BUTTON
        })))
        .respond_with(telegram_message_response())
        .expect(0)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_request_path(&telegram, "/bottest-token/SendMessage").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    drop(stale);

    for expected in ["Выберите задание или создайте новое:", "Выберите студента:"]
    {
        assert!(
            requests.contains(expected),
            "{expected} was never shown; requests:\n{requests}"
        );
    }
    assert!(
        !requests.contains(texts::STALE_BUTTON),
        "navigation must not be refused; requests:\n{requests}"
    );
}

/// А вот мутация со старой клавиатуры по-прежнему не проходит: один тап меняет задание.
#[tokio::test]
async fn a_mutation_from_an_older_message_is_still_refused() {
    let app = TestApp::start().await;
    let assignment = app.assignment();
    let updates = vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("{}{assignment}", callbacks::MANAGE_PREFIX),
            BOT_MESSAGE_ID,
        ),
        callback_update(
            3,
            app.teacher(),
            &format!("{}{assignment}", callbacks::ARCHIVE_PREFIX),
            BOT_MESSAGE_ID - 5,
        ),
    ];
    let telegram = telegram_server(updates).await;
    let stale = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": app.teacher(),
            "text": texts::STALE_BUTTON
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("stale mutation was not refused; requests:\n{requests}"));

    let status: String = sqlx::query_scalar("select status from assignments where id = $1")
        .bind(assignment)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(status, "published", "the assignment must not be archived");
}
