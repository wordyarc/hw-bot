use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::routing::post;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use wiremock::ResponseTemplate;

// Wiremock задаёт фиксированную задержку, а здесь тест управляет завершением ответа и видит
// отмену чтения клиентом. Redirect сохраняет запись исходного запроса в Wiremock.
pub struct ResponseGate {
    url: String,
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
    finished: Arc<Semaphore>,
    task: JoinHandle<()>,
}

struct BodyFinished(Arc<Semaphore>);

impl Drop for BodyFinished {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}

impl ResponseGate {
    pub async fn new(body: Value) -> Self {
        let started = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let finished = Arc::new(Semaphore::new(0));
        let router = Router::new().route(
            "/",
            post({
                let started = started.clone();
                let release = release.clone();
                let finished = finished.clone();
                move || {
                    let started = started.clone();
                    let release = release.clone();
                    let guard = BodyFinished(finished.clone());
                    let mut bytes = serde_json::to_vec(&body).unwrap();
                    let remaining = bytes.split_off(1);
                    async move {
                        let prefix =
                            futures::stream::iter([Ok::<_, std::io::Error>(Bytes::from(bytes))]);
                        let rest = futures::stream::once(async move {
                            let _guard = guard;
                            started.add_permits(1);
                            release
                                .acquire()
                                .await
                                .map_err(|_| std::io::ErrorKind::BrokenPipe)?
                                .forget();
                            Ok(Bytes::from(remaining))
                        });
                        (
                            [("content-type", "application/json")],
                            Body::from_stream(prefix.chain(rest)),
                        )
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            url,
            started,
            release,
            finished,
            task,
        }
    }

    pub fn response(&self) -> ResponseTemplate {
        ResponseTemplate::new(307).insert_header("location", self.url.as_str())
    }

    pub async fn wait_started(&self) {
        Self::wait(
            &self.started,
            "the client did not start reading the gated body",
        )
        .await;
    }

    pub fn release(&self) {
        self.release.add_permits(1);
    }

    pub async fn wait_finished(&self) {
        Self::wait(
            &self.finished,
            "the client neither completed nor cancelled the gated body",
        )
        .await;
    }

    async fn wait(semaphore: &Semaphore, message: &str) {
        tokio::time::timeout(Duration::from_secs(5), semaphore.acquire())
            .await
            .expect(message)
            .unwrap()
            .forget();
    }
}

impl Drop for ResponseGate {
    fn drop(&mut self) {
        self.release.close();
        self.task.abort();
    }
}
