use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use teloxide::net::Download;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{FileId, InputFile, Message, PhotoSize};
use teloxide::{ApiError, RequestError};
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::Instant;
use uuid::Uuid;

use super::errors::BotError;
use crate::app::{AppError, AppResult, FileService, OpenedFile};
use crate::domain::{FILE_NAME_MAX_LEN, FilePresentation, StoredFileMetadata};

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

/// Слот передачи. Скачать файл или загрузить его байты можно только через него, поэтому обойти
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
        self.acquire_until(Instant::now() + self.timeout).await
    }

    async fn acquire_until(&self, deadline: Instant) -> Result<FileTransfer<'_>, TransferError> {
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

    pub async fn send(
        &self,
        chat_id: ChatId,
        files: &FileService,
        file: &OpenedFile,
    ) -> Result<(), TransferError> {
        let deadline = Instant::now() + self.timeout;
        let metadata = file.metadata();
        if let Some(telegram_file_id) = file.telegram_file_id() {
            let input = InputFile::file_id(FileId(telegram_file_id.to_owned()));
            let resent = send_file(&self.bot, chat_id, metadata.presentation(), input);
            match within(deadline, resent).await {
                Ok(_) => return Ok(()),
                Err(TransferError::Telegram(error)) if is_rejected_file_id(&error) => {}
                Err(error) => return Err(error),
            }
        }
        let sent = self
            .acquire_until(deadline)
            .await?
            .upload(chat_id, metadata, || files.read_content(file))
            .await?;
        if let Some(telegram_file_id) = sent_file_id(&sent)
            && let Err(error) = files
                .remember_telegram_file_id(file, &telegram_file_id)
                .await
        {
            tracing::warn!(
                code = error.code().as_str(),
                "Telegram file id was not saved"
            );
        }
        Ok(())
    }
}

impl FileTransfer<'_> {
    pub async fn download(
        &self,
        file_id: &FileId,
        max_size: usize,
    ) -> Result<Vec<u8>, TransferError> {
        within(self.deadline, async {
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
    async fn upload<F>(
        &self,
        chat_id: ChatId,
        metadata: &StoredFileMetadata,
        read: impl FnOnce() -> F,
    ) -> Result<Message, TransferError>
    where
        F: Future<Output = AppResult<Vec<u8>>>,
    {
        within(self.deadline, async {
            let input =
                InputFile::memory(read().await?).file_name(metadata.filename().as_str().to_owned());
            send_file(self.bot, chat_id, metadata.presentation(), input).await
        })
        .await
    }
}

async fn within<T>(
    deadline: Instant,
    future: impl Future<Output = Result<T, TransferError>>,
) -> Result<T, TransferError> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| TransferError::Deadline)?
}

async fn send_file(
    bot: &Bot,
    chat_id: ChatId,
    presentation: FilePresentation,
    input: InputFile,
) -> Result<Message, TransferError> {
    Ok(match presentation {
        FilePresentation::Document => bot.send_document(chat_id, input).await?,
        FilePresentation::Image => bot.send_photo(chat_id, input).await?,
    })
}

fn is_rejected_file_id(error: &RequestError) -> bool {
    match error {
        RequestError::Api(
            ApiError::WrongFileId | ApiError::WrongFileIdOrUrl | ApiError::FileIdInvalid,
        ) => true,
        RequestError::Api(ApiError::Unknown(description)) => {
            description.starts_with("Bad Request: wrong remote file identifier specified")
        }
        _ => false,
    }
}

fn sent_file_id(message: &Message) -> Option<String> {
    let file = match message.document() {
        Some(document) => &document.file,
        None => &largest_photo(message.photo()?)?.file,
    };
    Some(file.id.0.clone())
}

fn largest_photo(photos: &[PhotoSize]) -> Option<&PhotoSize> {
    photos.iter().max_by_key(|photo| {
        (
            photo.file.size,
            u64::from(photo.width) * u64::from(photo.height),
        )
    })
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
    let photo = largest_photo(message.photo()?)?;
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

    #[test]
    fn only_a_definite_rejection_of_a_file_id_falls_back_to_the_upload() {
        for rejected in [
            ApiError::WrongFileIdOrUrl,
            ApiError::WrongFileId,
            ApiError::FileIdInvalid,
            ApiError::Unknown(
                "Bad Request: wrong remote file identifier specified: Wrong string length".into(),
            ),
        ] {
            assert!(is_rejected_file_id(&RequestError::Api(rejected)));
        }
        for unknown in [
            RequestError::RetryAfter(teloxide::types::Seconds::from_seconds(5)),
            RequestError::Io(Arc::new(std::io::Error::from(std::io::ErrorKind::TimedOut))),
            RequestError::Api(ApiError::BotBlocked),
            RequestError::Api(ApiError::InvalidToken),
        ] {
            assert!(!is_rejected_file_id(&unknown), "{unknown:?}");
        }
    }

    #[tokio::test(start_paused = true)]
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
