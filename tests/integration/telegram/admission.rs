use std::num::{NonZeroU32, NonZeroUsize};
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::harness::TestApp;
use crate::harness::telegram::*;

#[tokio::test]
async fn rate_limiter_silently_drops_updates_above_the_burst() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 1, Duration::ZERO).await;
    let settings = bot_settings_with(|settings| {
        settings.rate_limit_burst = NonZeroU32::MIN;
        settings.rate_limit_replenish = Duration::from_secs(60);
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the admitted update is answered");
    tokio::time::sleep(Duration::from_millis(100)).await;
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn rate_limiter_has_an_independent_bucket_per_user() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, Duration::ZERO).await;
    let settings = bot_settings_with(|settings| {
        settings.rate_limit_burst = NonZeroU32::MIN;
        settings.rate_limit_replenish = Duration::from_secs(60);
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("both users are answered");
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn concurrency_limit_backpressures_instead_of_dropping_updates() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, HELD_PERMIT).await;
    let settings =
        bot_settings_with(|settings| settings.max_concurrent_updates = NonZeroUsize::MIN);
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    wait_for_sent_message_count(&telegram, 1).await;
    tokio::time::sleep(HELD_PERMIT / 2).await;
    let while_held = sent_message_count(&telegram).await;
    let admitted = tokio::time::timeout(Duration::from_secs(3), sent.wait_until_satisfied()).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert_eq!(
        while_held, 1,
        "the single permit is still held by the first handler; requests:\n{requests}"
    );
    admitted.unwrap_or_else(|_| {
        panic!("the waiting update was dropped instead of admitted; requests:\n{requests}")
    });
}

#[tokio::test]
async fn a_waiting_update_holds_back_the_polling_offset() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, HELD_PERMIT).await;
    let settings =
        bot_settings_with(|settings| settings.max_concurrent_updates = NonZeroUsize::MIN);
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    wait_for_sent_message_count(&telegram, 1).await;
    let acknowledged_while_waiting = wait_for_poll_offset(&telegram, 3).await;
    let admitted = tokio::time::timeout(Duration::from_secs(3), sent.wait_until_satisfied()).await;
    let acknowledged_after_admission = wait_for_poll_offset(&telegram, 3).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    assert!(
        !acknowledged_while_waiting,
        "the batch must not be acknowledged while an update waits for a permit; requests:\n{requests}"
    );
    admitted
        .unwrap_or_else(|_| panic!("the waiting update was never admitted; requests:\n{requests}"));
    assert!(
        acknowledged_after_admission,
        "polling continues with the new offset once the permit is free; requests:\n{requests}"
    );
}

#[tokio::test]
async fn updates_from_one_chat_are_processed_sequentially() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, Duration::from_millis(200)).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);

    wait_for_sent_message_count(&telegram, 1).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(sent_message_count(&telegram).await, 1);
    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the second update runs after the first");
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn interactive_timeout_releases_the_permit_and_keeps_long_polling_alive() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = expect_sent_messages(&telegram, 2, Duration::from_millis(200)).await;
    let settings = bot_settings_with(|settings| {
        settings.request_timeout = Duration::from_millis(50);
        settings.max_concurrent_updates = NonZeroUsize::MIN;
    });
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    let both = tokio::time::timeout(Duration::from_secs(3), sent.wait_until_satisfied()).await;
    let polling = !task.is_finished();
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;

    both.unwrap_or_else(|_| {
        panic!("the timed out handler did not release its permit; requests:\n{requests}")
    });
    assert!(polling, "long polling must still be running");
}

#[tokio::test]
async fn a_server_error_cannot_hold_the_permit_during_teloxides_internal_sleep() {
    let app = TestApp::start().await;
    let telegram = telegram_server(vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.teacher(), 2, "/help"),
    ])
    .await;
    let sent = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_json(json!({"ok": false, "error_code": 500, "description": "failed"})),
        )
        .expect(2)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let (cancellation, mut task) = start_bot_with_settings(
        &app,
        &telegram,
        bot_settings_with(|settings| {
            settings.request_timeout = Duration::from_millis(100);
            settings.max_concurrent_updates = NonZeroUsize::MIN;
        }),
    );
    tokio::time::timeout(Duration::from_secs(2), sent.wait_until_satisfied())
        .await
        .expect("the second update does not wait for the ten-second library sleep");
    stop_bot(cancellation, &mut task).await;
}

/// teloxide обрабатывает остаток batch из буфера и подтверждает его последним polling-запросом:
/// новый offset, `limit = 1`, `timeout = 0`. Тест закрепляет это поведение библиотеки.
#[tokio::test]
async fn shutdown_finishes_while_an_update_waits_for_admission() {
    let app = TestApp::start().await;
    let updates = vec![
        message_update(1, app.teacher(), 1, "/help"),
        message_update(2, app.student(), 1, "/help"),
    ];
    let telegram = telegram_server(updates).await;
    let sent = Mock::given(method("POST"))
        .and(path("/bottest-token/SendMessage"))
        .respond_with(telegram_message_response().set_delay(HELD_PERMIT))
        .expect(1..)
        .with_priority(1)
        .mount_as_scoped(&telegram)
        .await;
    let settings =
        bot_settings_with(|settings| settings.max_concurrent_updates = NonZeroUsize::MIN);
    let (cancellation, mut task) = start_bot_with_settings(&app, &telegram, settings);

    wait_for_sent_message_count(&telegram, 1).await;
    let waiting = !wait_for_poll_offset(&telegram, 3).await;
    stop_bot(cancellation, &mut task).await;
    let requests = recorded_requests(&telegram).await;
    let last_poll = last_poll_request(&telegram).await;
    drop(sent);

    assert!(
        waiting,
        "the second update must still be waiting for a permit; requests:\n{requests}"
    );
    assert_eq!(
        sent_message_count(&telegram).await,
        2,
        "the buffered rest of the batch is drained during graceful shutdown; requests:\n{requests}"
    );
    assert_eq!(last_poll["offset"], json!(3));
    assert_eq!(last_poll["limit"], json!(1));
    assert_eq!(last_poll["timeout"], json!(0));
}

#[tokio::test]
async fn invalid_token_stops_the_telegram_bot() {
    let app = TestApp::start().await;
    let telegram = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bottest-token/GetMe"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false,
            "error_code": 401,
            "description": "Unauthorized"
        })))
        .expect(1)
        .mount(&telegram)
        .await;
    let (cancellation, task) = start_bot(&app, &telegram);

    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("invalid token stops the bot")
        .expect("Telegram task does not panic");

    assert!(matches!(
        result,
        Err(hw_bot::bot::TelegramBotError::InvalidCredentials)
    ));
    cancellation.cancel();
}

#[tokio::test]
async fn startup_and_dispatcher_share_one_get_me_response() {
    let app = TestApp::start().await;
    let telegram =
        telegram_server(vec![message_update(1, app.teacher(), 1, "/help@test_bot")]).await;
    let (cancellation, mut task) = start_bot(&app, &telegram);
    wait_for_request_count(&telegram, "/bottest-token/SendMessage", 1).await;
    stop_bot(cancellation, &mut task).await;
    let requests = telegram.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path() == "/bottest-token/GetMe")
            .count(),
        1
    );
    let help: serde_json::Value = requests
        .iter()
        .find(|request| request.url.path() == "/bottest-token/SendMessage")
        .unwrap()
        .body_json()
        .unwrap();
    assert!(help["text"].as_str().unwrap().starts_with('❓'));
    telegram.verify().await;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupStage {
    GetMe,
    GetWebhookInfo,
    DeleteWebhook,
    SetMyCommands,
    SetChatMenuButton,
}

impl StartupStage {
    fn path(self) -> &'static str {
        match self {
            Self::GetMe => "/bottest-token/GetMe",
            Self::GetWebhookInfo => "/bottest-token/GetWebhookInfo",
            Self::DeleteWebhook => "/bottest-token/DeleteWebhook",
            Self::SetMyCommands => "/bottest-token/SetMyCommands",
            Self::SetChatMenuButton => "/bottest-token/SetChatMenuButton",
        }
    }
}

#[tokio::test]
async fn cancellation_stops_every_network_stage_of_startup() {
    let app = TestApp::start().await;

    for stage in [
        StartupStage::GetMe,
        StartupStage::GetWebhookInfo,
        StartupStage::DeleteWebhook,
        StartupStage::SetMyCommands,
        StartupStage::SetChatMenuButton,
    ] {
        let telegram = MockServer::start().await;
        let webhook_url = (stage == StartupStage::DeleteWebhook).then_some("https://example.test");
        mount_startup_response(&telegram, StartupStage::GetMe, stage, telegram_me_body()).await;
        mount_startup_response(
            &telegram,
            StartupStage::GetWebhookInfo,
            stage,
            json!({
                "ok": true,
                "result": {
                    "url": webhook_url.unwrap_or(""),
                    "has_custom_certificate": false,
                    "pending_update_count": 0
                }
            }),
        )
        .await;
        for endpoint in [
            StartupStage::DeleteWebhook,
            StartupStage::SetMyCommands,
            StartupStage::SetChatMenuButton,
        ] {
            mount_startup_response(
                &telegram,
                endpoint,
                stage,
                json!({ "ok": true, "result": true }),
            )
            .await;
        }

        let (cancellation, task) = start_bot(&app, &telegram);
        wait_for_request_count(&telegram, stage.path(), 1).await;
        cancellation.cancel();

        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap_or_else(|_| panic!("cancellation at {stage:?} exceeded the grace"))
            .expect("Telegram task does not panic");
        result.unwrap_or_else(|error| panic!("cancellation at {stage:?} failed: {error}"));

        let requests = telegram.received_requests().await.unwrap();
        let paths: Vec<_> = requests.iter().map(|request| request.url.path()).collect();
        match stage {
            StartupStage::GetMe => assert!(!paths.contains(&StartupStage::GetWebhookInfo.path())),
            StartupStage::GetWebhookInfo | StartupStage::DeleteWebhook => {
                assert!(!paths.contains(&StartupStage::SetMyCommands.path()))
            }
            StartupStage::SetMyCommands => {
                assert!(!paths.contains(&StartupStage::SetChatMenuButton.path()))
            }
            StartupStage::SetChatMenuButton => assert_eq!(
                paths
                    .iter()
                    .filter(|path| **path == StartupStage::GetMe.path())
                    .count(),
                1,
                "dispatcher startup must not begin after cancellation"
            ),
        }
    }
}

async fn mount_startup_response(
    telegram: &MockServer,
    endpoint: StartupStage,
    delayed: StartupStage,
    body: serde_json::Value,
) {
    let response = ResponseTemplate::new(200).set_body_json(body);
    let response = if endpoint == delayed {
        response.set_delay(Duration::from_secs(5))
    } else {
        response
    };
    Mock::given(method("POST"))
        .and(path(endpoint.path()))
        .respond_with(response)
        .mount(telegram)
        .await;
}
