use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner,
    StoredFileParts, SubmissionId,
};
use crate::read_models::StoredFile;

const POSITION: &str = "stored_files.position";
const PRESENTATION: &str = "stored_files.presentation";
const SIZE_BYTES: &str = "stored_files.size_bytes";
const SHA256: &str = "stored_files.sha256";

macro_rules! select_stored_files {
    ($extra:literal, $tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query!(
            r#"
            select
                id, assignment_id, submission_id, position, presentation, filename, mime_type,
                size_bytes, sha256, created_at
            "# + $extra + $tail
            $(, $arg)*
        )
    };
}

macro_rules! stored_file_row {
    ($row:expr) => {
        StoredFileRow {
            id: $row.id,
            assignment_id: $row.assignment_id,
            submission_id: $row.submission_id,
            position: $row.position,
            presentation: $row.presentation,
            filename: $row.filename,
            mime_type: $row.mime_type,
            size_bytes: $row.size_bytes,
            sha256: $row.sha256,
            created_at: $row.created_at,
        }
    };
}

pub async fn insert_assignment_file<'e, E>(
    executor: E,
    metadata: &StoredFileMetadata,
    content: &[u8],
    telegram_file_id: Option<&str>,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let StoredFileOwner::Assignment(assignment_id) = metadata.owner() else {
        return Err(DbError::Precondition {
            message: "assignment file has a non-assignment owner",
        });
    };
    let sha256 = metadata.sha256();

    sqlx::query!(
        r#"
        insert into stored_files (
            id, assignment_id, position, presentation, filename, mime_type, size_bytes,
            sha256, content, telegram_file_id, created_at
        )
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
        metadata.id().as_uuid(),
        assignment_id.as_uuid(),
        checked_num_cast::<_, i64>(POSITION, metadata.position())?,
        metadata.presentation().as_str(),
        metadata.filename().as_str(),
        metadata.mime_type().map(|value| value.as_str()),
        checked_num_cast::<_, i64>(SIZE_BYTES, metadata.size_bytes())?,
        sha256.as_slice(),
        content,
        telegram_file_id,
        metadata.created_at(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn list_for_assignment<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
) -> DbResult<Vec<StoredFileMetadata>>
where
    E: PgExecutor<'e>,
{
    select_stored_files!(
        "",
        " from stored_files where assignment_id = $1 order by position",
        assignment_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| StoredFileMetadata::try_from(stored_file_row!(row)))
    .collect()
}

pub async fn find_assignment_file<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    file_id: StoredFileId,
) -> DbResult<Option<StoredFile>>
where
    E: PgExecutor<'e>,
{
    let row = select_stored_files!(
        ", telegram_file_id ",
        " from stored_files where assignment_id = $1 and id = $2",
        assignment_id.as_uuid(),
        file_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(|row| {
        Ok(StoredFile {
            metadata: StoredFileMetadata::try_from(stored_file_row!(row))?,
            telegram_file_id: row.telegram_file_id,
        })
    })
    .transpose()
}

pub async fn find_content<'e, E>(executor: E, file_id: StoredFileId) -> DbResult<Option<Vec<u8>>>
where
    E: PgExecutor<'e>,
{
    Ok(sqlx::query_scalar!(
        "select content from stored_files where id = $1",
        file_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?)
}

pub async fn remember_telegram_file_id<'e, E>(
    executor: E,
    file_id: StoredFileId,
    telegram_file_id: &str,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        "update stored_files set telegram_file_id = $2 where id = $1",
        file_id.as_uuid(),
        telegram_file_id,
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn delete_assignment_file<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    file_id: StoredFileId,
) -> DbResult<bool>
where
    E: PgExecutor<'e>,
{
    Ok(sqlx::query!(
        r#"
        delete from stored_files
        where assignment_id = $1
          and id = $2
        "#,
        assignment_id.as_uuid(),
        file_id.as_uuid(),
    )
    .execute(executor)
    .await?
    .rows_affected()
        == 1)
}

struct StoredFileRow {
    id: Uuid,
    assignment_id: Option<Uuid>,
    submission_id: Option<Uuid>,
    position: i64,
    presentation: String,
    filename: String,
    mime_type: Option<String>,
    size_bytes: i64,
    sha256: Vec<u8>,
    created_at: DateTime<Utc>,
}

impl TryFrom<StoredFileRow> for StoredFileMetadata {
    type Error = DbError;

    fn try_from(row: StoredFileRow) -> Result<Self, Self::Error> {
        let owner = match (row.assignment_id, row.submission_id) {
            (Some(id), None) => StoredFileOwner::Assignment(AssignmentId::from_uuid(id)),
            (None, Some(id)) => StoredFileOwner::Submission(SubmissionId::from_uuid(id)),
            _ => {
                return Err(DbError::invalid_value(
                    "stored_files.owner",
                    "invalid owner shape",
                ));
            }
        };
        let presentation = FilePresentation::parse_str(&row.presentation)
            .ok_or_else(|| DbError::invalid_value(PRESENTATION, &row.presentation))?;
        let sha256: [u8; 32] = row
            .sha256
            .try_into()
            .map_err(|value: Vec<u8>| DbError::invalid_value(SHA256, value.len().to_string()))?;

        Ok(StoredFileMetadata::restore(StoredFileParts {
            id: StoredFileId::from_uuid(row.id),
            owner,
            position: checked_num_cast(POSITION, row.position)?,
            presentation,
            filename: row.filename,
            mime_type: row.mime_type,
            size_bytes: checked_num_cast(SIZE_BYTES, row.size_bytes)?,
            sha256,
            created_at: row.created_at,
        })?)
    }
}
