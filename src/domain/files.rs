use chrono::{DateTime, Utc};

use crate::domain::{AssignmentId, DomainResult, NonEmptyString, StoredFileId, SubmissionId};
use crate::string_enum::str_enum;

pub const FILE_NAME_MAX_LEN: usize = 255;
pub const MIME_TYPE_MAX_LEN: usize = 255;

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum FilePresentation {
        Document => "document",
        Image => "image",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoredFileOwner {
    Assignment(AssignmentId),
    Submission(SubmissionId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredFileMetadata {
    id: StoredFileId,
    owner: StoredFileOwner,
    position: u64,
    presentation: FilePresentation,
    filename: NonEmptyString,
    mime_type: Option<NonEmptyString>,
    size_bytes: u64,
    sha256: [u8; 32],
    created_at: DateTime<Utc>,
}

pub(crate) struct StoredFileParts {
    pub id: StoredFileId,
    pub owner: StoredFileOwner,
    pub position: u64,
    pub presentation: FilePresentation,
    pub filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: u64,
    pub sha256: [u8; 32],
    pub created_at: DateTime<Utc>,
}

impl StoredFileMetadata {
    pub(crate) fn restore(parts: StoredFileParts) -> DomainResult<Self> {
        Ok(Self {
            id: parts.id,
            owner: parts.owner,
            position: parts.position,
            presentation: parts.presentation,
            filename: NonEmptyString::new_bounded(
                "file.filename",
                parts.filename,
                FILE_NAME_MAX_LEN,
            )?,
            mime_type: parts
                .mime_type
                .map(|value| {
                    NonEmptyString::new_bounded("file.mime_type", value, MIME_TYPE_MAX_LEN)
                })
                .transpose()?,
            size_bytes: parts.size_bytes,
            sha256: parts.sha256,
            created_at: parts.created_at,
        })
    }

    pub fn id(&self) -> StoredFileId {
        self.id
    }

    pub fn owner(&self) -> StoredFileOwner {
        self.owner
    }

    pub fn position(&self) -> u64 {
        self.position
    }

    pub fn presentation(&self) -> FilePresentation {
        self.presentation
    }

    pub fn filename(&self) -> &NonEmptyString {
        &self.filename
    }

    pub fn mime_type(&self) -> Option<&NonEmptyString> {
        self.mime_type.as_ref()
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub(crate) fn with_position(mut self, position: u64) -> Self {
        self.position = position;
        self
    }
}
