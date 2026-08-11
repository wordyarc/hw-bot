use anyhow::Context;
use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::{self, InMemStorage};
use teloxide::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::app::ApplicationContainer;

mod errors;
mod handlers;
mod state;
mod views;

use handlers::{Command, handle_callback, handle_command, handle_message};
use state::BotState;

type HandlerResult<T = ()> = anyhow::Result<T>;
type BotServices = ApplicationContainer;

#[derive(Clone)]
pub struct TelegramBot {
    bot: Bot,
    application: ApplicationContainer,
}

impl TelegramBot {
    pub fn new(
        application: ApplicationContainer,
        token: impl Into<String>,
        api_base_url: &str,
    ) -> anyhow::Result<Self> {
        let api_url = api_base_url
            .parse()
            .with_context(|| format!("invalid Telegram API base URL: {api_base_url}"))?;
        let bot = Bot::new(token).set_api_url(api_url);

        Ok(Self { bot, application })
    }

    pub async fn run(self, cancellation: CancellationToken) {
        let storage = InMemStorage::<BotState>::new();
        let mut dispatcher = Dispatcher::builder(self.bot, schema())
            .dependencies(dptree::deps![storage, self.application])
            .build();
        let shutdown = dispatcher.shutdown_token();
        let dispatch = dispatcher.dispatch();
        tokio::pin!(dispatch);

        tokio::select! {
            () = &mut dispatch => {},
            () = cancellation.cancelled() => {
                let _ = shutdown.shutdown();
                dispatch.await;
            }
        }
    }
}

fn schema() -> UpdateHandler<anyhow::Error> {
    let messages = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(dptree::endpoint(handle_message));
    let callbacks = Update::filter_callback_query().endpoint(handle_callback);

    dialogue::enter::<Update, InMemStorage<BotState>, BotState, _>()
        .branch(messages)
        .branch(callbacks)
}
