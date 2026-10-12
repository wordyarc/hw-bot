use std::time::Duration;

use hw_bot::app::ErrorCode;
use hw_bot::bot::{app_error_message, callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, body_partial_json, body_string_contains, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::TelegramScenario;

#[tokio::test]
async fn telegram_rejects_oversized_input_before_registration() {
    let app = TestApp::start().await;
    let user_id = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let updates = vec![
        message_update(1, user_id, 1, &format!("/start {token}")),
        message_update(2, user_id, 2, &"я".repeat(101)),
    ];
    let telegram = telegram_server(updates).await;
    let rejected = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        // Проверяем предел 100 на вводе из 101 символа; текст пояснения может меняться.
        .and(body_partial_json(json!({ "chat_id": user_id })))
        .and(body_string_contains("100"))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    tokio::time::timeout(Duration::from_secs(2), rejected.wait_until_satisfied())
        .await
        .expect("oversized input is rejected");
    stop_bot(cancellation, &mut task).await;
    let exists = sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from users where telegram_user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn start_without_an_invite_never_offers_a_group() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = telegram_server(vec![message_update(1, guest, 1, "/start")]).await;
    let refused = expect_guest(&telegram, guest).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), refused.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("guest was not asked for an invite; requests:\n{requests}"));
    assert!(
        !requests.contains(&app.fixture.group.code().as_str().to_owned()),
        "the guest must not see any group code; requests:\n{requests}"
    );
}

#[tokio::test]
async fn a_link_of_a_course_of_one_group_names_the_subject_and_the_group_and_registers() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, guest, 1, &format!("/start {token}")),
        message_update(2, guest, 2, "/help"),
        message_update(3, guest, 3, "Ivan"),
    ])
    .await;
    let prompt = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": guest,
            "text": texts::input_prompt(&texts::registration_prompt(
                app.fixture.subject.name().as_str(),
                app.fixture.group.code().as_str(),
            )),
            "reply_markup": { "remove_keyboard": true }
        })))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let asked = tokio::time::timeout(Duration::from_secs(5), prompt.wait_until_satisfied()).await;
    let registered = wait_for_group(&app.pool, guest).await;
    let commands_ready = wait_for_some(Duration::from_secs(5), || async {
        telegram
            .received_requests()
            .await
            .unwrap()
            .iter()
            .find_map(|request| {
                if !request.url.path().ends_with("/SetMyCommands") {
                    return None;
                }
                let body: serde_json::Value = request.body_json().unwrap();
                (body["scope"]["chat_id"] == guest
                    && body["commands"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|command| command["command"] == "tasks"))
                .then_some(())
            })
    })
    .await;
    assert!(
        commands_ready.is_some(),
        "student commands missing: {}",
        recorded_requests(&telegram).await
    );
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    asked.unwrap_or_else(|_| panic!("the group was not named; requests:\n{requests}"));
    assert_eq!(registered, Some(app.fixture.group.id().as_uuid()));
    let requests = telegram.received_requests().await.unwrap();
    let commands: Vec<serde_json::Value> = requests
        .iter()
        .filter(|request| request.url.path().ends_with("/SetMyCommands"))
        .map(|request| request.body_json().unwrap())
        .filter(|body: &serde_json::Value| body["scope"]["chat_id"] == guest)
        .collect();
    assert!(
        commands.last().unwrap()["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["command"] == "tasks")
    );
}

#[tokio::test]
async fn a_forged_and_a_rotated_link_answer_the_same() {
    let app = TestApp::start().await;
    let forged_user = app.next_telegram_id();
    let stale_user = app.next_telegram_id();
    let stale_token = app.course_invite_token().await;
    hw_bot::db::course_invites::rotate(
        &app.pool,
        app.fixture.course.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    let telegram = telegram_server(vec![
        message_update(
            1,
            forged_user,
            1,
            "/start ci_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        ),
        message_update(2, stale_user, 2, &format!("/start {stale_token}")),
    ])
    .await;
    let forged = expect_text(
        &telegram,
        forged_user,
        app_error_message(ErrorCode::InviteInvalid),
    )
    .await;
    let stale = expect_text(
        &telegram,
        stale_user,
        app_error_message(ErrorCode::InviteInvalid),
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let forged = tokio::time::timeout(Duration::from_secs(5), forged.wait_until_satisfied()).await;
    let stale = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    forged.unwrap_or_else(|_| panic!("forged link answered differently; requests:\n{requests}"));
    stale.unwrap_or_else(|_| panic!("rotated link answered differently; requests:\n{requests}"));
}

#[tokio::test]
async fn a_registered_student_keeps_their_group_when_opening_a_foreign_link() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let token = app.invite_token(foreign.course.id()).await;
    let telegram = telegram_server(vec![message_update(
        1,
        app.student(),
        1,
        &format!("/start {token}"),
    )])
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 1).await;
    stop_bot(cancellation, &mut task).await;
    assert_eq!(
        wait_for_group(&app.pool, app.student()).await,
        Some(app.fixture.group.id().as_uuid()),
        "an invite must not move an existing student"
    );
}

#[tokio::test]
async fn teacher_gets_a_forwardable_link_for_their_course() {
    let app = TestApp::start().await;
    let token = app.course_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("{}{}", callbacks::INVITE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let link = expect_text(
        &telegram,
        app.teacher(),
        &format!("https://t.me/test_bot?start={token}"),
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), link.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the link was not sent; requests:\n{requests}"));
}

#[tokio::test]
async fn a_teacher_of_another_course_gets_no_link_from_a_replayable_button() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let outsider = foreign.teacher.telegram_user_id().as_i64();
    let telegram = telegram_server(vec![
        message_update(1, outsider, 1, "/menu"),
        callback_update(
            2,
            outsider,
            &format!("{}{}", callbacks::INVITE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let denied = expect_text(
        &telegram,
        outsider,
        app_error_message(ErrorCode::PermissionDenied),
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), denied.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed
        .unwrap_or_else(|_| panic!("the foreign course was not refused; requests:\n{requests}"));
    assert!(
        !requests.contains("https://t.me/test_bot?start="),
        "no link may leak; requests:\n{requests}"
    );
}

#[tokio::test]
async fn a_confirmed_rotation_replaces_the_link() {
    let app = TestApp::start().await;
    let old_token = app.course_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("{}{}", callbacks::INVITE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(3, app.teacher(), callbacks::INVITE_ROTATE, BOT_MESSAGE_ID),
        callback_update(
            4,
            app.teacher(),
            callbacks::INVITE_CONFIRM_ROTATE,
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let replaced = expect_text(
        &telegram,
        app.teacher(),
        "♻️ Прежняя ссылка больше не работает.",
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed =
        tokio::time::timeout(Duration::from_secs(5), replaced.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("rotation did not happen; requests:\n{requests}"));

    let new_token = app.course_invite_token().await;
    assert_ne!(new_token, old_token);
    app.register_student(app.next_telegram_id(), "Late", &old_token)
        .await
        .error(axum::http::StatusCode::FORBIDDEN, "invite_invalid");
    app.register_student(app.next_telegram_id(), "Ivan", &new_token)
        .await
        .created();
}

#[tokio::test]
async fn a_stale_confirm_button_does_not_rotate_twice() {
    let app = TestApp::start().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/menu"),
        callback_update(
            2,
            app.teacher(),
            &format!("{}{}", callbacks::INVITE_PREFIX, app.course()),
            BOT_MESSAGE_ID,
        ),
        callback_update(3, app.teacher(), callbacks::INVITE_ROTATE, BOT_MESSAGE_ID),
        callback_update(
            4,
            app.teacher(),
            callbacks::INVITE_CONFIRM_ROTATE,
            BOT_MESSAGE_ID,
        ),
        callback_update(
            5,
            app.teacher(),
            callbacks::INVITE_CONFIRM_ROTATE,
            BOT_MESSAGE_ID,
        ),
    ])
    .await;
    let stale = expect_text(&telegram, app.teacher(), texts::STALE_BUTTON).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the stale confirm was accepted; requests:\n{requests}"));

    let generation: i64 = sqlx::query_scalar("select generation from teaching_course_invites")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(generation, 2, "only the first confirmation may rotate");
}

#[tokio::test]
async fn every_guest_entry_explains_access_and_removes_the_work_keyboard() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = telegram_server(vec![
        message_update(1, guest, 1, "/start"),
        message_update(2, guest, 2, "/help"),
        message_update(3, guest, 3, "/menu"),
        message_update(4, guest, 4, "/review"),
        callback_update(5, guest, callbacks::MENU_COURSES, BOT_MESSAGE_ID),
        message_update(6, guest, 6, "hello"),
    ])
    .await;
    let screen = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(
            json!({"chat_id": guest, "text": texts::GUEST_NEEDS_INVITE,
            "reply_markup": {"remove_keyboard": true}}),
        ))
        .respond_with(telegram_message_response())
        .expect(6)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    tokio::time::timeout(Duration::from_secs(5), screen.wait_until_satisfied())
        .await
        .unwrap();
    stop_bot(cancellation, &mut task).await;
    let requests = telegram.received_requests().await.unwrap();
    for request in requests
        .iter()
        .filter(|r| r.url.path().ends_with("/SetMyCommands"))
    {
        let body: serde_json::Value = request.body_json().unwrap();
        let commands = body["commands"].as_array().unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0]["command"], "start");
        assert_eq!(commands[1]["command"], "help");
    }
}

#[tokio::test]
async fn repeated_guest_text_uses_the_guest_screen_but_navigation_rechecks_access() {
    use crate::harness::telegram_scenario::TelegramScenario;
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(guest, "hello");
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(guest)
            .into_iter()
            .find(|message| message["text"] == texts::GUEST_NEEDS_INVITE)
    })
    .await
    .expect("guest screen");

    app.pool.close().await;
    for _ in 0..2 {
        telegram.message(guest, "hello again");
    }
    wait_for_some(Duration::from_secs(5), || async {
        let messages = telegram.messages(guest);
        (messages
            .iter()
            .filter(|message| message["text"] == texts::GUEST_NEEDS_INVITE)
            .count()
            == 3)
            .then_some(())
    })
    .await
    .expect("guest replies must not need the database");
    telegram.message(guest, "/menu");
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(guest)
            .into_iter()
            .find(|message| message["text"] == app_error_message(ErrorCode::InternalError))
    })
    .await
    .expect("navigation must recheck access");
    stop_bot(cancellation, &mut task).await;

    let requests = telegram.server.received_requests().await.unwrap();
    let command_updates = requests
        .iter()
        .filter(|request| {
            request.url.path().ends_with("/SetMyCommands")
                && request.body_json::<serde_json::Value>().unwrap()["scope"]["chat_id"] == guest
        })
        .count();
    assert_eq!(command_updates, 1);
}

#[tokio::test]
async fn losing_registration_replaces_the_previous_teacher_commands() {
    use crate::harness::telegram_scenario::TelegramScenario;
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.teacher(), "/menu");
    telegram.flush(app.teacher()).await;
    sqlx::query!(
        "update users set telegram_user_id = $2 where telegram_user_id = $1",
        app.teacher(),
        app.next_telegram_id()
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let screen = expect_guest(&telegram.server, app.teacher()).await;
    telegram.message(app.teacher(), "/menu");
    tokio::time::timeout(Duration::from_secs(5), screen.wait_until_satisfied())
        .await
        .unwrap();
    stop_bot(cancellation, &mut task).await;
    let commands: Vec<serde_json::Value> = telegram
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/SetMyCommands"))
        .map(|r| r.body_json().unwrap())
        .filter(|body: &serde_json::Value| body["scope"]["chat_id"] == app.teacher())
        .collect();
    assert!(
        commands.first().unwrap()["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command["command"] == "review"),
        "{commands:?}"
    );
    assert_eq!(
        commands.last().unwrap()["commands"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(commands.last().unwrap()["commands"][0]["command"], "start");
}

#[tokio::test]
async fn a_command_hint_failure_does_not_hide_the_guest_access_instructions() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let telegram = telegram_server(vec![message_update(1, guest, 1, "/start")]).await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/SetMyCommands"))
        .and(body_partial_json(json!({ "scope": {"chat_id": guest} })))
        .respond_with(wiremock::ResponseTemplate::new(400).set_body_json(json!({
            "ok": false, "error_code": 400, "description": "Bad Request: injected failure"
        })))
        .expect(1)
        .with_priority(1)
        .mount(&telegram)
        .await;
    let screen = expect_guest(&telegram, guest).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    tokio::time::timeout(Duration::from_secs(5), screen.wait_until_satisfied())
        .await
        .unwrap();
    stop_bot(cancellation, &mut task).await;
    telegram.verify().await;
}

#[tokio::test]
async fn help_does_not_turn_a_database_failure_into_missing_registration() {
    let app = TestApp::start().await;
    let telegram = telegram_server(vec![message_update(1, app.teacher(), 1, "/help")]).await;
    app.pool.close().await;
    let failed = expect_text(
        &telegram,
        app.teacher(),
        app_error_message(ErrorCode::InternalError),
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    tokio::time::timeout(Duration::from_secs(5), failed.wait_until_satisfied())
        .await
        .unwrap();
    stop_bot(cancellation, &mut task).await;
    assert!(
        !recorded_requests(&telegram)
            .await
            .contains(texts::GUEST_NEEDS_INVITE)
    );
}

async fn expect_guest(telegram: &wiremock::MockServer, guest: i64) -> wiremock::MockGuard {
    Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(
            json!({ "chat_id": guest, "text": texts::GUEST_NEEDS_INVITE,
            "reply_markup": { "remove_keyboard": true } }),
        ))
        .respond_with(telegram_message_response())
        .expect(1)
        .with_priority(1)
        .mount_as_scoped(telegram)
        .await
}

async fn wait_for_text(telegram: &TelegramScenario, user: i64, text: &str) -> serde_json::Value {
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(user)
            .into_iter()
            .find(|message| message["text"] == text)
    })
    .await
    .unwrap_or_else(|| panic!("{text} was not sent to {user}"))
}

async fn is_registered(app: &TestApp, user: i64) -> bool {
    sqlx::query_scalar("select exists(select 1 from users where telegram_user_id = $1)")
        .bind(user)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_link_of_a_course_of_several_groups_asks_for_the_group_before_the_name() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    let choice = telegram
        .button(
            guest,
            &cb(callbacks::REGISTRATION_GROUP_PREFIX, second.id().as_uuid()),
        )
        .await;
    let offered: Vec<String> = telegram
        .messages(guest)
        .into_iter()
        .find(|message| message["message_id"] == choice.message_id)
        .unwrap()["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap())
        .map(|button| button["text"].as_str().unwrap().to_owned())
        .collect();
    telegram.click(guest, &choice);
    let prompt = texts::input_prompt(&texts::registration_prompt(
        app.fixture.subject.name().as_str(),
        second.code().as_str(),
    ));
    wait_for_text(&telegram, guest, &prompt).await;
    telegram.message(guest, "Ivan");
    let registered = wait_for_group(&app.pool, guest).await;
    stop_bot(cancellation, &mut task).await;

    let mut expected = vec![
        app.fixture.group.code().as_str().to_owned(),
        second.code().as_str().to_owned(),
    ];
    expected.sort_unstable();
    assert_eq!(offered, expected);
    let choice_message = telegram
        .messages(guest)
        .into_iter()
        .find(|message| message["message_id"] == choice.message_id)
        .unwrap();
    assert!(
        choice_message["reply_markup"].is_null(),
        "the group buttons are removed after the choice"
    );
    assert_eq!(registered, Some(second.id().as_uuid()));
}

#[tokio::test]
async fn a_group_button_of_a_replaced_choice_is_stale() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let data = cb(callbacks::REGISTRATION_GROUP_PREFIX, second.id().as_uuid());
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    let replaced = telegram.button(guest, &data).await;
    telegram.message(guest, &format!("/start {token}"));
    telegram
        .button_after(guest, &data, replaced.message_id)
        .await;
    telegram.click(guest, &replaced);
    wait_for_text(&telegram, guest, texts::STALE_BUTTON).await;
    telegram.message(guest, "Ivan");
    wait_for_text(&telegram, guest, texts::CHOOSE_BUTTON).await;
    stop_bot(cancellation, &mut task).await;

    assert!(!is_registered(&app, guest).await);
}

#[tokio::test]
async fn a_link_rotated_between_the_group_and_the_name_does_not_register() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    let choice = telegram
        .button(
            guest,
            &cb(callbacks::REGISTRATION_GROUP_PREFIX, second.id().as_uuid()),
        )
        .await;
    telegram.click(guest, &choice);
    let prompt = texts::input_prompt(&texts::registration_prompt(
        app.fixture.subject.name().as_str(),
        second.code().as_str(),
    ));
    wait_for_text(&telegram, guest, &prompt).await;
    hw_bot::db::course_invites::rotate(
        &app.pool,
        app.fixture.course.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    telegram.message(guest, "Ivan");
    wait_for_text(
        &telegram,
        guest,
        app_error_message(ErrorCode::InviteInvalid),
    )
    .await;
    stop_bot(cancellation, &mut task).await;

    assert!(!is_registered(&app, guest).await);
}

#[tokio::test]
async fn a_link_rotated_before_the_name_ends_the_name_step() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    let prompt = texts::input_prompt(&texts::registration_prompt(
        app.fixture.subject.name().as_str(),
        app.fixture.group.code().as_str(),
    ));
    wait_for_text(&telegram, guest, &prompt).await;
    hw_bot::db::course_invites::rotate(
        &app.pool,
        app.fixture.course.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    telegram.message(guest, "Ivan");
    wait_for_text(
        &telegram,
        guest,
        app_error_message(ErrorCode::InviteInvalid),
    )
    .await;
    let before = telegram.messages(guest).len();
    telegram.message(guest, "Ivan");
    let reply = wait_for_some(Duration::from_secs(5), || async {
        telegram.messages(guest).get(before).cloned()
    })
    .await
    .expect("the second name is answered");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(reply["text"], texts::GUEST_NEEDS_INVITE);
}

#[tokio::test]
async fn the_link_screen_names_the_subject_and_every_group_of_the_course() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(app.teacher(), "/courses");
    telegram
        .click_button(app.teacher(), &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;
    telegram
        .click_button(app.teacher(), &cb(callbacks::INVITE_PREFIX, app.course()))
        .await;
    let mut codes = vec![app.fixture.group.code().as_str(), second.code().as_str()];
    codes.sort_unstable();
    let header = texts::invite_header(app.fixture.subject.name().as_str(), &codes);
    wait_for_text(&telegram, app.teacher(), &header).await;
    wait_for_text(&telegram, app.teacher(), texts::INVITE_REUSE_HINT).await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_link_rotated_before_the_group_is_chosen_ends_the_group_step() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    let group = telegram
        .button(
            guest,
            &cb(callbacks::REGISTRATION_GROUP_PREFIX, second.id().as_uuid()),
        )
        .await;
    hw_bot::db::course_invites::rotate(
        &app.pool,
        app.fixture.course.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    telegram.click(guest, &group);
    wait_for_text(
        &telegram,
        guest,
        app_error_message(ErrorCode::InviteInvalid),
    )
    .await;
    let before = telegram.messages(guest).len();
    telegram.message(guest, "Ivan");
    let reply = wait_for_some(Duration::from_secs(5), || async {
        telegram.messages(guest).get(before).cloned()
    })
    .await
    .expect("the text after the refusal is answered");
    stop_bot(cancellation, &mut task).await;

    assert_eq!(reply["text"], texts::GUEST_NEEDS_INVITE);
}

#[tokio::test]
async fn a_registration_whose_user_cannot_be_read_again_still_opens_the_home_screen() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let token = app.course_invite_token().await;
    app.rename_table_after_writing("users", "student_groups")
        .await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);

    telegram.message(guest, &format!("/start {token}"));
    telegram.message(guest, "Ivan");
    let home = texts::home_greeting("Ivan");
    let failure = app_error_message(ErrorCode::InternalError);
    wait_for_some(Duration::from_secs(5), || async {
        telegram
            .messages(guest)
            .into_iter()
            .find(|message| message["text"] == home.as_str() || message["text"] == failure)
    })
    .await
    .expect("the registration is answered");
    stop_bot(cancellation, &mut task).await;

    let replies = telegram.messages(guest);
    let home = replies
        .iter()
        .find(|message| message["text"] == home)
        .unwrap_or_else(|| panic!("the home screen is shown: {replies:#?}"));
    assert!(home["reply_markup"]["keyboard"].is_array(), "{home:#?}");
    assert!(
        !replies.iter().any(|message| message["text"] == failure),
        "{replies:#?}"
    );
}
