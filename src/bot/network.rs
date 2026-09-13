use std::future::IntoFuture;
use std::time::Duration;

use futures::future::BoxFuture;
use teloxide::payloads;
use teloxide::requests::{HasPayload, JsonRequest, Output, Request, Requester};
use teloxide::types::{BotCommand, CallbackQueryId, ChatId, MessageId};

// Только интерактивные методы: polling и обмен файлами имеют собственные бюджеты.
#[derive(Clone)]
pub(super) struct Bot {
    pub inner: teloxide::Bot,
    timeout: Duration,
}

impl Bot {
    pub fn new(inner: teloxide::Bot, timeout: Duration) -> Self {
        Self { inner, timeout }
    }

    fn timed<R>(&self, request: R) -> TimedRequest<R> {
        TimedRequest {
            request,
            timeout: self.timeout,
        }
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

    pub fn set_my_commands(
        &self,
        commands: Vec<BotCommand>,
    ) -> TimedRequest<JsonRequest<payloads::SetMyCommands>> {
        self.timed(self.inner.set_my_commands(commands))
    }

    pub fn set_chat_menu_button(&self) -> TimedRequest<JsonRequest<payloads::SetChatMenuButton>> {
        self.timed(self.inner.set_chat_menu_button())
    }

    pub fn get_me(&self) -> TimedRequest<JsonRequest<payloads::GetMe>> {
        self.timed(self.inner.get_me())
    }
}

pub(super) struct TimedRequest<R> {
    request: R,
    timeout: Duration,
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
    // reqwest не ограничивает sleep внутри teloxide после HTTP 5xx.
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
