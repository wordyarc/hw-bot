use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use teloxide::RequestError;
use teloxide::net::Download;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{FileId, InputFile, Message};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;
use uuid::Uuid;

use super::errors::BotError;
use crate::app::{AppError, AppResult, StoredFile};
use crate::domain::{FILE_NAME_MAX_LEN, FilePresentation};

#[derive(Debug, thiserror::Error)]
pub(super) enum TransferError {
    #[error("file transfer deadline exceeded")]
    Deadline,

    #[error("downloaded file exceeds the configured limit")]
    TooLarge,

    #[error("file transfer semaphore closed")]
    Closed,

    #[error("Telegram file download failed")]
    Download(#[source] reqwest::Error),

    #[error(transparent)]
    Telegram(#[from] RequestError),

    #[error(transparent)]
    App(#[from] AppError),
}

impl From<TransferError> for BotError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::Telegram(error) => Self::Telegram(error),
            TransferError::App(error) => Self::App(error),
            other => Self::Other(other.into()),
        }
    }
}

#[derive(Clone)]
pub(super) struct FileTransfers {
    bot: Bot,
    semaphore: Arc<Semaphore>,
    timeout: Duration,
}

/// Слот передачи. Скачать или отправить файл можно только через него, поэтому обойти
/// предел параллельных передач и общий deadline одной передачи нельзя.
pub(super) struct FileTransfer<'a> {
    bot: &'a Bot,
    _permit: SemaphorePermit<'a>,
    deadline: Instant,
}

pub(super) struct IncomingFile {
    pub file_id: FileId,
    pub declared_size: Option<usize>,
    pub presentation: FilePresentation,
    pub filename: String,
    pub mime_type: Option<String>,
}

impl FileTransfers {
    pub fn new(bot: Bot, max_concurrent: usize, timeout: Duration) -> Self {
        Self {
            bot,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            timeout,
        }
    }

    pub async fn acquire(&self) -> Result<FileTransfer<'_>, TransferError> {
        let deadline = Instant::now() + self.timeout;
        let permit = tokio::time::timeout_at(deadline, self.semaphore.acquire())
            .await
            .map_err(|_| TransferError::Deadline)?
            .map_err(|_| TransferError::Closed)?;

        Ok(FileTransfer {
            bot: &self.bot,
            _permit: permit,
            deadline,
        })
    }
}

impl FileTransfer<'_> {
    pub async fn download(
        &self,
        file_id: &FileId,
        max_size: usize,
    ) -> Result<Vec<u8>, TransferError> {
        self.within(async {
            let file = self.bot.get_file(file_id.clone()).await?;
            let mut stream = self.bot.download_file_stream(&file.path);
            let mut content = Vec::with_capacity((file.size as usize).min(max_size));
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(TransferError::Download)?;
                if content.len().saturating_add(chunk.len()) > max_size {
                    return Err(TransferError::TooLarge);
                }
                content.extend_from_slice(&chunk);
            }
            Ok(content)
        })
        .await
    }

    /// Чтение файла из БД входит в тот же deadline, что и отправка, поэтому файл открывает
    /// переданное замыкание.
    pub async fn send<F>(
        &self,
        chat_id: ChatId,
        open: impl FnOnce() -> F,
    ) -> Result<(), TransferError>
    where
        F: Future<Output = AppResult<StoredFile>>,
    {
        self.within(async {
            let file = open().await?;
            let input = InputFile::memory(file.content)
                .file_name(file.metadata.filename().as_str().to_owned());
            match file.metadata.presentation() {
                FilePresentation::Document => {
                    self.bot.send_document(chat_id, input).await?;
                }
                FilePresentation::Image => {
                    self.bot.send_photo(chat_id, input).await?;
                }
            }
            Ok(())
        })
        .await
    }

    async fn within<T>(
        &self,
        future: impl Future<Output = Result<T, TransferError>>,
    ) -> Result<T, TransferError> {
        tokio::time::timeout_at(self.deadline, future)
            .await
            .map_err(|_| TransferError::Deadline)?
    }
}

pub(super) fn incoming_file(message: &Message) -> Option<IncomingFile> {
    if let Some(document) = message.document() {
        let filename = document
            .file_name
            .as_deref()
            .map(safe_filename)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("document-{}.bin", Uuid::now_v7()));
        return Some(IncomingFile {
            file_id: document.file.id.clone(),
            declared_size: declared_file_size(document.file.size),
            presentation: FilePresentation::Document,
            filename,
            mime_type: document.mime_type.as_ref().map(ToString::to_string),
        });
    }
    let photo = message.photo()?.iter().max_by_key(|photo| {
        (
            photo.file.size,
            u64::from(photo.width) * u64::from(photo.height),
        )
    })?;
    Some(IncomingFile {
        file_id: photo.file.id.clone(),
        declared_size: declared_file_size(photo.file.size),
        presentation: FilePresentation::Image,
        filename: format!("photo-{}.jpg", Uuid::now_v7()),
        mime_type: Some("image/jpeg".to_owned()),
    })
}

fn declared_file_size(size: u32) -> Option<usize> {
    (size != u32::MAX).then_some(size as usize)
}

fn safe_filename(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '/' | '\\') {
                '_'
            } else {
                character
            }
        })
        .take(FILE_NAME_MAX_LEN)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_is_safe_and_bounded() {
        let filename = format!(" /unsafe\\{}\0.txt ", "я".repeat(FILE_NAME_MAX_LEN));
        let safe = safe_filename(&filename);

        assert!(!safe.contains(['/', '\\', '\0']));
        assert_eq!(safe.chars().count(), FILE_NAME_MAX_LEN);
    }

    #[tokio::test]
    async fn waiting_for_a_transfer_slot_has_a_deadline_and_does_not_leak_permits() {
        let transfers = FileTransfers::new(Bot::new("test-token"), 1, Duration::from_millis(50));
        let first = transfers.acquire().await.unwrap();
        assert!(matches!(
            transfers.acquire().await,
            Err(TransferError::Deadline)
        ));
        drop(first);
        assert!(transfers.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn get_file_and_download_share_one_deadline() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/bottest-token/GetFile"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(300))
                .set_body_json(serde_json::json!({"ok": true, "result": {
                    "file_id": "file", "file_unique_id": "unique", "file_size": 3, "file_path": "document.bin"
                }}))).expect(1).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/file/bottest-token/document.bin"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(300))
                    .set_body_bytes(b"abc"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let transfers = FileTransfers::new(
            Bot::new("test-token").set_api_url(server.uri().parse().unwrap()),
            1,
            Duration::from_millis(500),
        );
        let permit = transfers.acquire().await.unwrap();
        let result = permit.download(&FileId("file".into()), 10).await;
        assert!(matches!(result, Err(TransferError::Deadline)));
        drop(permit);
        assert!(transfers.acquire().await.is_ok());
    }

    #[tokio::test]
    async fn a_stream_longer_than_the_limit_is_reported_as_too_large() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bottest-token/GetFile"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {
                    "file_id": "file", "file_unique_id": "unique", "file_size": 2,
                    "file_path": "document.bin"
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/file/bottest-token/document.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc"))
            .mount(&server)
            .await;
        let transfers = FileTransfers::new(
            Bot::new("test-token").set_api_url(server.uri().parse().unwrap()),
            1,
            Duration::from_secs(5),
        );
        let permit = transfers.acquire().await.unwrap();

        let result = permit.download(&FileId("file".into()), 2).await;

        assert!(matches!(result, Err(TransferError::TooLarge)));
    }

    #[tokio::test]
    async fn a_slow_body_stream_is_cancelled_after_its_first_chunk() {
        use axum::{
            Json, Router,
            body::{Body, Bytes},
            routing::{get, post},
        };
        use std::sync::atomic::{AtomicUsize, Ordering};

        let chunks = Arc::new(AtomicUsize::new(0));
        let sent = chunks.clone();
        let router = Router::new()
            .route(
                "/bottest-token/GetFile",
                post(|| async {
                    Json(serde_json::json!({"ok": true, "result": {
                        "file_id": "file", "file_unique_id": "unique", "file_size": 2,
                        "file_path": "document.bin"
                    }}))
                }),
            )
            .route(
                "/file/bottest-token/document.bin",
                get(move || {
                    let sent = sent.clone();
                    async move {
                        Body::from_stream(futures::stream::unfold(
                            (0, sent),
                            |(index, sent)| async move {
                                if index == 2 {
                                    return None;
                                }
                                if index > 0 {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                                sent.fetch_add(1, Ordering::Relaxed);
                                Some((
                                    Ok::<_, std::convert::Infallible>(Bytes::from_static(b"x")),
                                    (index + 1, sent),
                                ))
                            },
                        ))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let transfers = FileTransfers::new(
            Bot::new("test-token").set_api_url(url.parse().unwrap()),
            1,
            Duration::from_millis(500),
        );
        let permit = transfers.acquire().await.unwrap();
        let result = permit.download(&FileId("file".into()), 10).await;
        server.abort();
        assert!(matches!(result, Err(TransferError::Deadline)));
        assert_eq!(chunks.load(Ordering::Relaxed), 1);
        drop(permit);
        assert!(transfers.acquire().await.is_ok());
    }
}
