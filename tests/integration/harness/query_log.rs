use std::sync::{Arc, LazyLock, Mutex};

use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, NoSubscriber};
use tracing::{Dispatch, Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

static SECOND_DISPATCHER: LazyLock<Dispatch> = LazyLock::new(|| Dispatch::new(NoSubscriber::new()));

#[derive(Clone, Default)]
pub struct QueryLog(Arc<Mutex<Vec<String>>>);

impl QueryLog {
    pub fn capture() -> (Self, DefaultGuard) {
        LazyLock::force(&SECOND_DISPATCHER);
        let log = Self::default();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(log.clone()));
        (log, guard)
    }

    pub async fn during<T>(&self, action: impl Future<Output = T>) -> (T, Vec<String>) {
        self.take();
        let result = action.await;
        (result, self.take())
    }

    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl<S: Subscriber> Layer<S> for QueryLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() != "sqlx::query" {
            return;
        }
        let mut summary = Summary::default();
        event.record(&mut summary);
        self.0.lock().unwrap().push(summary.0);
    }
}

#[derive(Default)]
struct Summary(String);

impl Visit for Summary {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "summary" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "summary" {
            self.0 = format!("{value:?}");
        }
    }
}
