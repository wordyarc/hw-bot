use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::telegram::{
    callback_update, message_update, mount_startup_mocks, telegram_me_body, wait_for_some,
};

#[derive(Default)]
struct State {
    updates: Vec<Value>,
    messages: Vec<Value>,
    next_message: i32,
}

pub struct TelegramScenario {
    pub server: MockServer,
    state: Arc<Mutex<State>>,
}

#[derive(Clone)]
pub struct Button {
    pub message_id: i32,
    data: String,
    message: Value,
}

impl TelegramScenario {
    pub async fn new() -> Self {
        let server = MockServer::start().await;
        let state = Arc::new(Mutex::new(State {
            next_message: 1000,
            ..State::default()
        }));
        mount_startup_mocks(&server).await;
        Mock::given(method("POST"))
            .and(path("/bottest-token/GetMe"))
            .respond_with(ResponseTemplate::new(200).set_body_json(telegram_me_body()))
            .mount(&server)
            .await;
        let polling = state.clone();
        Mock::given(method("POST"))
            .and(path("/bottest-token/GetUpdates"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let offset = body["offset"].as_u64().unwrap_or(0);
                let state = polling.lock().unwrap();
                let updates: Vec<_> = state
                    .updates
                    .iter()
                    .filter(|update| update["update_id"].as_u64().unwrap() >= offset)
                    .cloned()
                    .collect();
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(20))
                    .set_body_json(json!({"ok": true, "result": updates}))
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/bottest-token/AnswerCallbackQuery"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": true})),
            )
            .with_priority(10)
            .mount(&server)
            .await;
        let messages = state.clone();
        Mock::given(method("POST"))
            .and(path("/bottest-token/SendMessage"))
            .respond_with(move |request: &wiremock::Request| {
                let body: Value = request.body_json().unwrap();
                let mut state = messages.lock().unwrap();
                state.next_message += 1;
                let message = json!({
                    "message_id": state.next_message, "date": 1,
                    "chat": {"id": body["chat_id"], "type": "private"},
                    "from": {"id": 999, "is_bot": true, "first_name": "Test Bot"},
                    "text": body["text"], "reply_markup": body["reply_markup"],
                });
                state.messages.push(message.clone());
                // ReplyKeyboardMarkup в ответ Message не входит.
                let mut result = message;
                if result["reply_markup"]["inline_keyboard"].is_null() {
                    result.as_object_mut().unwrap().remove("reply_markup");
                }
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result}))
            })
            .with_priority(10)
            .mount(&server)
            .await;
        Self { server, state }
    }

    pub fn message(&self, user: i64, text: &str) {
        let mut state = self.state.lock().unwrap();
        let id = state.updates.len() as u32 + 1;
        state
            .updates
            .push(message_update(id, user, id as i32, text));
    }

    pub async fn button(&self, user: i64, data: &str) -> Button {
        self.button_after(user, data, 0).await
    }

    pub async fn button_after(&self, user: i64, data: &str, previous: i32) -> Button {
        wait_for_some(Duration::from_secs(5), || async {
            let state = self.state.lock().unwrap();
            state.messages.iter().rev().find_map(|message| {
                let id = message["message_id"].as_i64()? as i32;
                if message["chat"]["id"] != user || id <= previous {
                    return None;
                }
                let rows = message["reply_markup"]["inline_keyboard"].as_array()?;
                rows.iter()
                    .flat_map(|row| row.as_array().unwrap())
                    .find_map(|button| {
                        (button["callback_data"] == data).then(|| Button {
                            message_id: id,
                            data: data.to_owned(),
                            message: message.clone(),
                        })
                    })
            })
        })
        .await
        .unwrap_or_else(|| panic!("button {data} was not sent to {user}"))
    }

    pub fn click(&self, user: i64, button: &Button) {
        let mut state = self.state.lock().unwrap();
        let id = state.updates.len() as u32 + 1;
        let mut update = callback_update(id, user, &button.data, button.message_id);
        update["callback_query"]["message"] = button.message.clone();
        state.updates.push(update);
    }

    pub async fn click_button(&self, user: i64, data: &str) -> Button {
        let button = self.button(user, data).await;
        self.click(user, &button);
        button
    }

    pub async fn flush(&self, user: i64) {
        let previous = self.state.lock().unwrap().next_message;
        self.message(user, "/help");
        wait_for_some(Duration::from_secs(5), || async {
            self.state
                .lock()
                .unwrap()
                .messages
                .iter()
                .find(|message| {
                    message["chat"]["id"] == user
                        && message["message_id"].as_i64().unwrap() > i64::from(previous)
                        && message["text"]
                            .as_str()
                            .is_some_and(|text| text.starts_with('❓'))
                })
                .cloned()
        })
        .await
        .expect("the same-chat barrier was answered");
    }
}
