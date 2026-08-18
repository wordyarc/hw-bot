use std::time::Duration;

use hw_bot::app::ErrorCode;
use hw_bot::bot::{app_error_message, callbacks, texts};
use serde_json::json;
use wiremock::Mock;
use wiremock::matchers::{body_json, body_partial_json, body_string_contains, method, path};

use crate::harness::TestApp;
use crate::harness::telegram::*;

#[tokio::test]
async fn telegram_rejects_oversized_input_before_registration() {
    let app = TestApp::start().await;
    let user_id = app.next_telegram_id();
    let token = app.group_invite_token().await;
    let updates = vec![
        message_update(1, user_id, 1, &format!("/start {token}")),
        message_update(2, user_id, 2, &"я".repeat(101)),
    ];
    let telegram = telegram_server(updates).await;
    let rejected = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        // Формулировка не контракт, а вот предел 100 - да: тест шлёт ровно 101 символ
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
    let refused = expect_text(&telegram, guest, texts::GUEST_NEEDS_INVITE).await;
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
async fn a_valid_link_names_the_group_and_registers_the_student() {
    let app = TestApp::start().await;
    let guest = app.next_telegram_id();
    let token = app.group_invite_token().await;
    let telegram = telegram_server(vec![
        message_update(1, guest, 1, &format!("/start {token}")),
        message_update(2, guest, 2, "Ivan"),
    ])
    .await;
    let prompt = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .and(body_json(json!({
            "chat_id": guest,
            "text": format!(
                "Регистрация в группу {}.\n\nКак вас называть?\n\n/cancel — отменить",
                app.fixture.group.code().as_str()
            ),
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
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    asked.unwrap_or_else(|_| panic!("the group was not named; requests:\n{requests}"));
    assert_eq!(registered, Some(app.fixture.group.id().as_uuid()));
}

/// Битая подпись и уже отозванная ссылка обязаны выглядеть для гостя одинаково.
#[tokio::test]
async fn a_forged_and_a_rotated_link_answer_the_same() {
    let app = TestApp::start().await;
    let forged_user = app.next_telegram_id();
    let stale_user = app.next_telegram_id();
    let stale_token = app.group_invite_token().await;
    hw_bot::db::group_invites::rotate(
        &app.pool,
        app.fixture.group.id(),
        hw_bot::domain::InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    let telegram = telegram_server(vec![
        message_update(
            1,
            forged_user,
            1,
            "/start gi_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
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
    let token = app.invite_token(foreign.group.id()).await;
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
    let token = app.group_invite_token().await;
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
    let old_token = app.group_invite_token().await;
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

    let new_token = app.group_invite_token().await;
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
    let stale = expect_text(
        &telegram,
        app.teacher(),
        "Эта кнопка из старого сообщения, действие не выполнено. Откройте /menu.",
    )
    .await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    let observed = tokio::time::timeout(Duration::from_secs(5), stale.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    observed.unwrap_or_else(|_| panic!("the stale confirm was accepted; requests:\n{requests}"));

    let generation: i64 = sqlx::query_scalar("select generation from student_group_invites")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(generation, 2, "only the first confirmation may rotate");
}
