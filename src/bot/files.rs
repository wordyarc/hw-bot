use anyhow::{Context, anyhow};
use futures::StreamExt;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::{Bot, ChatId, Requester};
use teloxide::types::{FileId, InputFile, Message};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::app::OpenedFile;
use crate::domain::{FILE_NAME_MAX_LEN, FilePresentation};

use super::HandlerResult;

#[derive(Clone)]
pub(super) struct FileTransfers {
    bot: Bot,
    semaphore: Arc<Semaphore>,
}

pub(super) struct IncomingFile {
    pub file_id: FileId,
    pub declared_size: Option<usize>,
    pub presentation: FilePresentation,
    pub filename: String,
    pub mime_type: Option<String>,
}

impl FileTransfers {
    pub fn new(bot: Bot, max_concurrent: usize) -> Self {
        Self {
            bot,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    pub async fn acquire(&self) -> HandlerResult<OwnedSemaphorePermit> {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .context("file transfer semaphore closed")
    }

    pub async fn download(&self, file_id: &FileId, max_size: usize) -> HandlerResult<Vec<u8>> {
        let file = self.bot.get_file(file_id.clone()).await?;
        let mut stream = self.bot.download_file_stream(&file.path);
        let mut content = Vec::with_capacity((file.size as usize).min(max_size));
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let size = content
                .len()
                .checked_add(chunk.len())
                .context("downloaded file size overflow")?;
            if size > max_size {
                return Err(anyhow!("downloaded file exceeds configured limit"));
            }
            content.extend_from_slice(&chunk);
        }
        Ok(content)
    }

    pub async fn send(&self, chat_id: ChatId, file: OpenedFile) -> HandlerResult {
        let input =
            InputFile::memory(file.content).file_name(file.metadata.filename().as_str().to_owned());
        match file.metadata.presentation() {
            FilePresentation::Document => {
                self.bot.send_document(chat_id, input).await?;
            }
            FilePresentation::Image => {
                self.bot.send_photo(chat_id, input).await?;
            }
        }
        Ok(())
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
}
