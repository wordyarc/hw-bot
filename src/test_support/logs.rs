use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};

use serde_json::Value;
use tracing::Dispatch;
use tracing::subscriber::{DefaultGuard, NoSubscriber};

static SECOND_DISPATCHER: LazyLock<Dispatch> = LazyLock::new(|| Dispatch::new(NoSubscriber::new()));

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogBuffer {
    pub(crate) fn of(log: impl FnOnce()) -> Self {
        let buffer = Self::default();
        let _capture = buffer.capture();
        log();
        buffer
    }

    pub(crate) fn capture(&self) -> DefaultGuard {
        LazyLock::force(&SECOND_DISPATCHER);
        let writer = self.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_env_filter("hw_bot=info,tower_http=info")
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::set_default(subscriber)
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    pub(crate) fn events(&self) -> Vec<Value> {
        serde_json::Deserializer::from_slice(&self.0.lock().unwrap())
            .into_iter::<Value>()
            .map(Result::unwrap)
            .collect()
    }
}
