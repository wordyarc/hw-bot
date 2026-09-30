use std::future::IntoFuture;
use std::time::Duration;

use futures::future::BoxFuture;
use teloxide::adaptors::{CacheMe, cache_me::CachedMeRequest};
use teloxide::payloads;
use teloxide::requests::{HasPayload, JsonRequest, Output, Request, Requester, RequesterExt};
use teloxide::types::{BotCommand, CallbackQueryId, ChatId, MessageId};

// Бюджет интерактивных запросов. Для polling и передачи файлов таймауты задаются отдельно.
#[derive(Clone)]
pub(crate) struct Bot {
    pub inner: CacheMe<teloxide::Bot>,
    timeout: Duration,
}

impl Bot {
    pub fn new(inner: teloxide::Bot, timeout: Duration) -> Self {
        Self {
            inner: inner.cache_me(),
            timeout,
        }
    }

    fn timed<R>(&self, request: R) -> TimedRequest<R> {
        TimedRequest::new(request, self.timeout)
    }

    pub fn send_message(
        &self,
        chat_id: ChatId,
        text: impl Into<String>,
    ) -> TimedRequest<JsonRequest<payloads::SendMessage>> {
        self.timed(self.inner.send_message(chat_id, text))
    }

    pub fn answer_callback_query(
        &self,
        id: CallbackQueryId,
    ) -> TimedRequest<JsonRequest<payloads::AnswerCallbackQuery>> {
        self.timed(self.inner.answer_callback_query(id))
    }

    pub fn edit_message_reply_markup(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
    ) -> TimedRequest<JsonRequest<payloads::EditMessageReplyMarkup>> {
        self.timed(self.inner.edit_message_reply_markup(chat_id, message_id))
    }

    pub fn edit_message_text(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
        text: impl Into<String>,
    ) -> TimedRequest<JsonRequest<payloads::EditMessageText>> {
        self.timed(self.inner.edit_message_text(chat_id, message_id, text))
    }

    pub fn set_my_commands(
        &self,
        commands: Vec<BotCommand>,
    ) -> TimedRequest<JsonRequest<payloads::SetMyCommands>> {
        self.timed(self.inner.set_my_commands(commands))
    }

    pub fn set_chat_menu_button(&self) -> TimedRequest<JsonRequest<payloads::SetChatMenuButton>> {
        self.timed(self.inner.set_chat_menu_button())
    }

    pub fn get_me(&self) -> TimedRequest<CachedMeRequest<JsonRequest<payloads::GetMe>>> {
        self.timed(self.inner.get_me())
    }

    pub fn get_chat(&self, chat_id: ChatId) -> TimedRequest<JsonRequest<payloads::GetChat>> {
        self.timed(self.inner.get_chat(chat_id))
    }
}

pub(crate) struct TimedRequest<R> {
    request: R,
    timeout: Duration,
}

impl<R> TimedRequest<R> {
    pub(crate) fn new(request: R, timeout: Duration) -> Self {
        Self { request, timeout }
    }
}

impl<R: HasPayload> HasPayload for TimedRequest<R> {
    type Payload = R::Payload;

    fn payload_ref(&self) -> &Self::Payload {
        self.request.payload_ref()
    }
    fn payload_mut(&mut self) -> &mut Self::Payload {
        self.request.payload_mut()
    }
}

impl<R> Request for TimedRequest<R>
where
    R: Request<Err = teloxide::RequestError>,
    R::Send: 'static,
    R::SendRef: 'static,
    Output<R>: 'static,
{
    type Err = teloxide::RequestError;
    type Send = BoxFuture<'static, Result<Output<R>, Self::Err>>;
    type SendRef = Self::Send;

    fn send(self) -> Self::Send {
        Box::pin(with_deadline(self.timeout, self.request.send()))
    }

    fn send_ref(&self) -> Self::SendRef {
        Box::pin(with_deadline(self.timeout, self.request.send_ref()))
    }
}

impl<R> IntoFuture for TimedRequest<R>
where
    Self: Request,
{
    type Output = Result<Output<Self>, <Self as Request>::Err>;
    type IntoFuture = <Self as Request>::Send;

    fn into_future(self) -> Self::IntoFuture {
        self.send()
    }
}

async fn with_deadline<T>(
    timeout: Duration,
    future: impl Future<Output = Result<T, teloxide::RequestError>>,
) -> Result<T, teloxide::RequestError> {
    // После HTTP 5xx teloxide делает sleep, на который таймаут reqwest уже не распространяется.
    tokio::time::timeout(timeout, future)
        .await
        .unwrap_or_else(|_| {
            Err(teloxide::RequestError::Io(std::sync::Arc::new(
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Telegram request deadline exceeded",
                ),
            )))
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use teloxide::types::ChatFullInfo;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::test_support::telegram::private_chat_info;

    #[tokio::test]
    async fn get_chat_sends_the_id_and_reads_full_chat_info() {
        let server = MockServer::start().await;
        let info = private_chat_info(12345, Some("student_name"), true);
        Mock::given(method("POST"))
            .and(path("/bot123:test/GetChat"))
            .and(body_json(json!({ "chat_id": 12345 })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true, "result": info
            })))
            .expect(1)
            .mount(&server)
            .await;
        let inner =
            crate::telegram::new_bot("123:test", &server.uri(), Duration::from_secs(5)).unwrap();
        let bot = Bot::new(inner, Duration::from_secs(1));

        let actual = bot.get_chat(ChatId(12345)).await.unwrap();

        assert_eq!(
            actual,
            serde_json::from_value::<ChatFullInfo>(info).unwrap()
        );
    }

    #[tokio::test]
    async fn get_chat_obeys_the_full_interactive_budget() {
        let server = MockServer::start().await;
        Mock::given(path("/bot123:test/GetChat"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(json!({
                        "ok": true, "result": private_chat_info(12345, None, false)
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let inner =
            crate::telegram::new_bot("123:test", &server.uri(), Duration::from_secs(5)).unwrap();
        let bot = Bot::new(inner, Duration::from_millis(100));

        let error = tokio::time::timeout(Duration::from_secs(1), bot.get_chat(ChatId(12345)))
            .await
            .expect("interactive deadline precedes the HTTP client timeout")
            .unwrap_err();

        assert!(matches!(error, teloxide::RequestError::Io(error)
            if error.kind() == std::io::ErrorKind::TimedOut));
    }
}
