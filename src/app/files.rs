use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};

use crate::app::access::load_managing_teacher_on;
use crate::app::assignments::load_visible_assignment_for_student;
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    Assignment, AssignmentId, FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner,
    StoredFileParts,
};

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
pub struct OpenedFile {
    pub metadata: StoredFileMetadata,
    pub content: Vec<u8>,
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
        let (assignment, revision) = self
            .load_managed_assignment(actor, assignment_id, "view assignment files")
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
        let (assignment, _) = load_versioned_managed_assignment(
            &mut tx,
            command.actor,
            command.assignment_id,
            "add assignment file",
        )
        .await?;
        let revision =
            advance_revision(&mut tx, assignment.id(), command.expected_revision).await?;
        let files = db::files::list_for_assignment(&mut *tx, assignment.id()).await?;
        if files.len() >= self.policy.max_files_per_assignment {
            return Err(AppError::AssignmentFileLimitReached {
                max_files: self.policy.max_files_per_assignment,
            });
        }
        let position = first_free_position(&files)?;
        let metadata = metadata.with_position(position);
        db::files::insert_assignment_file(&mut *tx, &metadata, &command.content).await?;
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
        let (assignment, _) = load_versioned_managed_assignment(
            &mut tx,
            command.actor,
            command.assignment_id,
            "delete assignment file",
        )
        .await?;
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
        self.load_managed_assignment(actor, assignment_id, "open assignment file")
            .await?;
        let file = db::files::find_assignment_file(&self.pool, assignment_id, file_id)
            .await?
            .required("file")?;

        Ok(OpenedFile {
            metadata: file.metadata,
            content: file.content,
        })
    }

    pub async fn open_assignment_file_for_student(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
    ) -> AppResult<OpenedFile> {
        load_visible_assignment_for_student(&self.pool, actor, assignment_id).await?;
        let file = db::files::find_assignment_file(&self.pool, assignment_id, file_id)
            .await?
            .required("file")?;

        Ok(OpenedFile {
            metadata: file.metadata,
            content: file.content,
        })
    }

    async fn load_managed_assignment(
        &self,
        actor: Actor,
        assignment_id: AssignmentId,
        action: &'static str,
    ) -> AppResult<(Assignment, i64)> {
        let mut connection = self.pool.acquire().await?;
        let (assignment, revision) =
            load_versioned_managed_assignment(&mut connection, actor, assignment_id, action)
                .await?;
        Ok((assignment, revision))
    }
}

async fn advance_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    assignment_id: AssignmentId,
    expected_revision: i64,
) -> AppResult<i64> {
    match db::assignments::advance_revision(&mut **tx, assignment_id, expected_revision).await {
        Err(db::DbError::Conflict { .. }) => Err(AppError::AssignmentChangedConcurrently),
        result => Ok(result?),
    }
}

async fn load_versioned_managed_assignment(
    connection: &mut PgConnection,
    actor: Actor,
    assignment_id: AssignmentId,
    action: &'static str,
) -> AppResult<(Assignment, i64)> {
    let stored = db::assignments::find_versioned_by_id(&mut *connection, assignment_id)
        .await?
        .required("assignment")?;
    let revision = stored.revision();
    let assignment = stored.into_assignment();
    let (actor, course) = load_managing_teacher_on(
        &mut *connection,
        actor,
        assignment.teaching_course_id(),
        action,
    )
    .await?;
    assignment.ensure_managed_by(&course, &actor, action)?;

    Ok((assignment, revision))
}

fn first_free_position(files: &[StoredFileMetadata]) -> AppResult<u64> {
    let mut position = 1_u64;
    for file in files {
        if file.position() == position {
            position = position
                .checked_add(1)
                .ok_or(AppError::AssignmentFileLimitReached {
                    max_files: files.len(),
                })?;
        } else if file.position() > position {
            break;
        }
    }
    Ok(position)
}
