use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::app::access::{load_managed_versioned_assignment, load_managed_versioned_assignment_on};
use crate::app::assignments::load_visible_assignment_for_student;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner,
    StoredFileParts,
};
use crate::read_models::{StoredFile, VersionedAssignment};

#[derive(Clone, Copy, Debug)]
pub struct FilePolicy {
    pub max_file_size_bytes: usize,
    pub max_files_per_assignment: usize,
    pub max_files_per_submission: usize,
}

#[derive(Clone, Debug)]
pub struct AssignmentFiles {
    pub assignment: Assignment,
    pub revision: i64,
    pub files: Vec<StoredFileMetadata>,
}

#[derive(Clone, Debug)]
pub struct AddAssignmentFile {
    pub assignment_id: AssignmentId,
    pub actor: Actor,
    pub expected_revision: i64,
    pub presentation: FilePresentation,
    pub filename: String,
    pub mime_type: Option<String>,
    pub telegram_file_id: Option<String>,
    pub content: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct DeleteAssignmentFile {
    pub assignment_id: AssignmentId,
    pub file_id: StoredFileId,
    pub actor: Actor,
    pub expected_revision: i64,
}

#[derive(Clone, Debug)]
pub struct OpenedFile(StoredFile);

impl OpenedFile {
    pub fn metadata(&self) -> &StoredFileMetadata {
        &self.0.metadata
    }

    pub fn telegram_file_id(&self) -> Option<&str> {
        self.0.telegram_file_id.as_deref()
    }
}

#[derive(Clone, Debug)]
pub struct FileService {
    pool: PgPool,
    policy: FilePolicy,
}

impl FileService {
    pub fn new(pool: PgPool, policy: FilePolicy) -> Self {
        Self { pool, policy }
    }

    pub fn policy(&self) -> FilePolicy {
        self.policy
    }

    pub async fn list_assignment_files_for_teacher(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
    ) -> AppResult<AssignmentFiles> {
        let VersionedAssignment {
            assignment,
            revision,
        } = load_managed_versioned_assignment(
            &self.pool,
            actor,
            assignment_id,
            "view assignment files",
        )
        .await?;
        let files = db::files::list_for_assignment(&self.pool, assignment_id).await?;

        Ok(AssignmentFiles {
            assignment,
            revision,
            files,
        })
    }

    pub async fn add_assignment_file(
        &self,
        command: AddAssignmentFile,
    ) -> AppResult<AssignmentFiles> {
        let actual_bytes = command.content.len();
        if actual_bytes > self.policy.max_file_size_bytes {
            return Err(AppError::FileTooLarge {
                max_bytes: self.policy.max_file_size_bytes,
                actual_bytes,
            });
        }
        let size_bytes = u64::try_from(actual_bytes).map_err(|_| AppError::FileTooLarge {
            max_bytes: self.policy.max_file_size_bytes,
            actual_bytes,
        })?;
        let sha256: [u8; 32] = Sha256::digest(&command.content).into();
        let metadata = StoredFileMetadata::restore(StoredFileParts {
            id: StoredFileId::new(),
            owner: StoredFileOwner::Assignment(command.assignment_id),
            position: 1,
            presentation: command.presentation,
            filename: command.filename,
            mime_type: command.mime_type,
            size_bytes,
            sha256,
            created_at: Utc::now(),
        })?;

        let mut tx = self.pool.begin().await?;
        let assignment = load_managed_versioned_assignment_on(
            &mut tx,
            command.actor,
            command.assignment_id,
            "add assignment file",
        )
        .await?
        .assignment;
        let revision =
            advance_revision(&mut tx, assignment.id(), command.expected_revision).await?;
        let files = db::files::list_for_assignment(&mut *tx, assignment.id()).await?;
        if files.len() >= self.policy.max_files_per_assignment {
            return Err(AppError::AssignmentFileLimitReached {
                max_files: self.policy.max_files_per_assignment,
            });
        }
        let position = first_free_position(&files);
        let metadata = metadata.with_position(position);
        db::files::insert_assignment_file(
            &mut *tx,
            &metadata,
            &command.content,
            command.telegram_file_id.as_deref(),
        )
        .await?;
        tx.commit().await?;

        let mut files = files;
        files.push(metadata);
        files.sort_by_key(StoredFileMetadata::position);
        Ok(AssignmentFiles {
            assignment,
            revision,
            files,
        })
    }

    pub async fn delete_assignment_file(
        &self,
        command: DeleteAssignmentFile,
    ) -> AppResult<AssignmentFiles> {
        let mut tx = self.pool.begin().await?;
        let assignment = load_managed_versioned_assignment_on(
            &mut tx,
            command.actor,
            command.assignment_id,
            "delete assignment file",
        )
        .await?
        .assignment;
        let revision =
            advance_revision(&mut tx, assignment.id(), command.expected_revision).await?;
        if !db::files::delete_assignment_file(&mut *tx, assignment.id(), command.file_id).await? {
            return Err(AppError::NotFound { resource: "file" });
        }
        let files = db::files::list_for_assignment(&mut *tx, assignment.id()).await?;
        tx.commit().await?;

        Ok(AssignmentFiles {
            assignment,
            revision,
            files,
        })
    }

    pub async fn open_assignment_file_for_teacher(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
    ) -> AppResult<OpenedFile> {
        load_managed_versioned_assignment(&self.pool, actor, assignment_id, "open assignment file")
            .await?;
        self.open_assignment_file(assignment_id, file_id).await
    }

    pub async fn open_assignment_file_for_student(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
    ) -> AppResult<OpenedFile> {
        load_visible_assignment_for_student(&self.pool, actor, assignment_id).await?;
        self.open_assignment_file(assignment_id, file_id).await
    }

    pub async fn read_content(&self, file: &OpenedFile) -> AppResult<Vec<u8>> {
        db::files::find_content(&self.pool, file.metadata().id())
            .await?
            .required("file")
    }

    pub async fn remember_telegram_file_id(
        &self,
        file: &OpenedFile,
        telegram_file_id: &str,
    ) -> AppResult<()> {
        db::files::remember_telegram_file_id(&self.pool, file.metadata().id(), telegram_file_id)
            .await?;
        Ok(())
    }

    async fn open_assignment_file(
        &self,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
    ) -> AppResult<OpenedFile> {
        db::files::find_assignment_file(&self.pool, assignment_id, file_id)
            .await?
            .required("file")
            .map(OpenedFile)
    }
}

async fn advance_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    assignment_id: AssignmentId,
    expected_revision: i64,
) -> AppResult<i64> {
    super::on_conflict(
        db::assignments::advance_revision(&mut **tx, assignment_id, expected_revision).await,
        AppError::AssignmentChangedConcurrently,
    )
}

fn first_free_position(files: &[StoredFileMetadata]) -> u64 {
    let mut position = 1_u64;
    for file in files {
        if file.position() == position {
            position += 1;
        } else if file.position() > position {
            break;
        }
    }
    position
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::t0;

    #[test]
    fn an_attachment_reuses_the_first_gap_in_position_order() {
        for (positions, expected) in [
            (vec![], 1),
            (vec![1, 2, 3], 4),
            (vec![2, 3], 1),
            (vec![1, 3, 4], 2),
        ] {
            let files: Vec<_> = positions
                .iter()
                .map(|&position| {
                    StoredFileMetadata::restore(StoredFileParts {
                        id: StoredFileId::new(),
                        owner: StoredFileOwner::Assignment(AssignmentId::new()),
                        position,
                        presentation: FilePresentation::Document,
                        filename: "file.txt".into(),
                        mime_type: None,
                        size_bytes: 0,
                        sha256: [0; 32],
                        created_at: t0(),
                    })
                    .unwrap()
                })
                .collect();
            assert_eq!(
                first_free_position(&files),
                expected,
                "positions: {positions:?}"
            );
        }
    }
}
