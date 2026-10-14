use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use hw_bot::bot::texts;
use hw_bot::domain::USER_TEXT_MAX_LEN;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::telegram::{
    callback_update, document_update, message_update, mount_startup_mocks, telegram_me_body,
    wait_for_some,
};

const TELEGRAM_MESSAGE_LIMIT: usize = 4096;

pub fn text_over_the_limit_once_its_link_is_revealed() -> Value {
    json!({
        "text": "a".repeat(USER_TEXT_MAX_LEN - 10),
        "entities": [{
            "type": "text_link",
            "offset": 0,
            "length": 5,
            "url": "https://example.test/a-hidden-link"
        }]
    })
}

pub fn callbacks_of(message: &Value) -> Vec<String> {
    message["reply_markup"]["inline_keyboard"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row.as_array().unwrap().clone())
        .filter_map(|button| button["callback_data"].as_str().map(str::to_owned))
        .collect()
}

#[derive(Default)]
struct State {
    updates: Vec<Value>,
    messages: Vec<Value>,
    next_message: i32,
    acknowledged_offset: u64,
}

impl State {
    fn next_message_id(&mut self) -> i32 {
        self.next_message += 1;
        self.next_message
    }
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

impl Button {
    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn with_data(mut self, data: String) -> Self {
        self.data = data;
        self
    }

    pub fn in_chat(mut self, user: i64) -> Self {
        self.message["chat"]["id"] = user.into();
        self
    }
}

impl TelegramScenario {
    pub fn messages(&self, user: i64) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .messages
            .iter()
            .filter(|message| message["chat"]["id"] == user)
            .cloned()
            .collect()
    }

    pub fn message_by_id(&self, user: i64, id: i32) -> Value {
        self.messages(user)
            .into_iter()
            .find(|message| message["message_id"] == id)
            .unwrap_or_else(|| panic!("message {id} was not sent to {user}"))
    }

    pub fn texts(&self, user: i64) -> Vec<String> {
        self.messages(user)
            .iter()
            .filter_map(|message| message["text"].as_str().map(str::to_owned))
            .collect()
    }

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
                let mut state = polling.lock().unwrap();
                state.acknowledged_offset = state.acknowledged_offset.max(offset);
                let updates: Vec<_> = state
                    .updates
                    .iter()
                    .filter(|update| {
                        update["update_id"].as_u64().unwrap() >= state.acknowledged_offset
                    })
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
                if body["text"]
                    .as_str()
                    .is_some_and(|text| text.encode_utf16().count() > TELEGRAM_MESSAGE_LIMIT)
                {
                    return ResponseTemplate::new(400).set_body_json(json!({
                        "ok": false,
                        "error_code": 400,
                        "description": "Bad Request: message is too long"
                    }));
                }
                let mut state = messages.lock().unwrap();
                let message = json!({
                    "message_id": state.next_message_id(), "date": Utc::now().timestamp(),
                    "chat": {"id": body["chat_id"], "type": "private"},
                    "from": {"id": 999, "is_bot": true, "first_name": "Test Bot"},
                    "text": body["text"], "reply_markup": body["reply_markup"],
                });
                state.messages.push(message.clone());
                // Bot API не возвращает ReplyKeyboardMarkup в составе Message.
                let mut result = message;
                if result["reply_markup"]["inline_keyboard"].is_null() {
                    result.as_object_mut().unwrap().remove("reply_markup");
                }
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result}))
            })
            .with_priority(10)
            .mount(&server)
            .await;
        for endpoint in ["EditMessageReplyMarkup", "EditMessageText"] {
            let messages = state.clone();
            Mock::given(method("POST"))
                .and(path(format!("/bottest-token/{endpoint}")))
                .respond_with(move |request: &wiremock::Request| {
                    let body: Value = request.body_json().unwrap();
                    let mut state = messages.lock().unwrap();
                    let message = state
                        .messages
                        .iter_mut()
                        .find(|message| {
                            message["chat"]["id"] == body["chat_id"]
                                && message["message_id"] == body["message_id"]
                        })
                        .expect("editing a message sent by the bot");
                    if body["reply_markup"].is_null() {
                        message.as_object_mut().unwrap().remove("reply_markup");
                    } else {
                        message["reply_markup"] = body["reply_markup"].clone();
                    }
                    if !body["text"].is_null() {
                        message["text"] = body["text"].clone();
                    }
                    ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": message}))
                })
                .with_priority(10)
                .mount(&server)
                .await;
        }
        Self { server, state }
    }

    pub fn message(&self, user: i64, text: &str) {
        let mut state = self.state.lock().unwrap();
        let id = state.updates.len() as u32 + 1;
        let message_id = state.next_message_id();
        state
            .updates
            .push(message_update(id, user, message_id, text));
    }

    pub fn message_with(&self, user: i64, fields: Value) {
        let mut state = self.state.lock().unwrap();
        let id = state.updates.len() as u32 + 1;
        let message_id = state.next_message_id();
        let mut update = message_update(id, user, message_id, "");
        let message = update["message"].as_object_mut().unwrap();
        message.remove("text");
        message.extend(fields.as_object().unwrap().clone());
        state.updates.push(update);
    }

    pub fn document(&self, user: i64, file_id: &str, filename: &str, size: usize) {
        let mut state = self.state.lock().unwrap();
        let id = state.updates.len() as u32 + 1;
        let message_id = state.next_message_id();
        state.updates.push(document_update(
            id, user, message_id, file_id, filename, size,
        ));
    }

    pub async fn button(&self, user: i64, data: &str) -> Button {
        self.button_after(user, data, 0).await
    }

    pub async fn button_with_text(&self, user: i64, text: &str) -> Button {
        self.wait_for_labelled(user, |label| label == text)
            .await
            .unwrap_or_else(|| panic!("button {text} was not sent to {user}"))
    }

    pub async fn preset(&self, user: i64, label: &str) -> Button {
        let prefix = texts::date_preset(label, "");
        self.wait_for_labelled(user, |text| text.starts_with(&prefix))
            .await
            .unwrap_or_else(|| panic!("preset {label} was not sent to {user}"))
    }

    pub async fn click_preset(&self, user: i64, label: &str) -> Button {
        let button = self.preset(user, label).await;
        self.click(user, &button);
        self.flush(user).await;
        button
    }

    async fn wait_for_labelled(&self, user: i64, matches: impl Fn(&str) -> bool) -> Option<Button> {
        wait_for_some(Duration::from_secs(5), || async {
            let state = self.state.lock().unwrap();
            state.messages.iter().rev().find_map(|message| {
                if message["chat"]["id"] != user {
                    return None;
                }
                message["reply_markup"]["inline_keyboard"]
                    .as_array()?
                    .iter()
                    .flat_map(|row| row.as_array().unwrap())
                    .find_map(|button| {
                        button["text"]
                            .as_str()
                            .is_some_and(&matches)
                            .then(|| Button {
                                message_id: message["message_id"].as_i64().unwrap() as i32,
                                data: button["callback_data"].as_str().unwrap().to_owned(),
                                message: message.clone(),
                            })
                    })
            })
        })
        .await
    }

    pub async fn button_after(&self, user: i64, data: &str, previous: i32) -> Button {
        self.wait_for_button(user, previous, |callback| callback == data)
            .await
            .unwrap_or_else(|| panic!("button {data} was not sent to {user}"))
    }

    pub async fn button_matching(&self, user: i64, matches: impl Fn(&str) -> bool) -> Button {
        self.wait_for_button(user, 0, matches)
            .await
            .unwrap_or_else(|| panic!("no matching button was sent to {user}"))
    }

    async fn wait_for_button(
        &self,
        user: i64,
        previous: i32,
        matches: impl Fn(&str) -> bool,
    ) -> Option<Button> {
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
                    .filter_map(|button| button["callback_data"].as_str())
                    .find(|callback| matches(callback))
                    .map(|callback| Button {
                        message_id: id,
                        data: callback.to_owned(),
                        message: message.clone(),
                    })
            })
        })
        .await
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
        // ID редактируемого сообщения прежний. Ждём смены состояния, чтобы не принять старую кнопку за новую.
        self.flush(user).await;
        button
    }

    pub async fn flush(&self, user: i64) {
        self.message(user, "/help");
        let help_titles: Vec<String> = [texts::student_help(), texts::teacher_help()]
            .iter()
            .filter_map(|help| help.lines().next().map(str::to_owned))
            .collect();
        wait_for_some(Duration::from_secs(5), || async {
            let state = self.state.lock().unwrap();
            let requested = state
                .updates
                .iter()
                .filter(|update| {
                    let message = &update["message"];
                    message["chat"]["id"] == user
                        && (message["text"] == "/help" || message["text"] == texts::BUTTON_HELP)
                })
                .count();
            let answered = state
                .messages
                .iter()
                .filter(|message| {
                    message["chat"]["id"] == user
                        && message["text"].as_str().is_some_and(|text| {
                            help_titles
                                .iter()
                                .any(|title| text.starts_with(title.as_str()))
                        })
                })
                .count();
            (answered >= requested).then_some(())
        })
        .await
        .expect("the same-chat barrier was answered");
    }
}
