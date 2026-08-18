use sqlx::PgPool;
use sqlx::Row;
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::time::Duration;

pub mod academic;
pub mod assignments;
pub mod attempts;
mod bootstrap;
mod convert;
mod enums;
pub(crate) mod error;
pub mod files;
pub mod group_invites;
pub mod notifications;
pub mod submissions;
pub mod users;

pub use bootstrap::{BootstrapError, run_bootstrap};
pub use error::{DbError, DbResult};

pub(crate) use convert::checked_num_cast;
pub(crate) use error::ensure_updated;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

pub async fn connect_with_max_connections(
    database_url: &str,
    max_connections: u32,
) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

pub async fn connect_runtime(
    database_url: &str,
    max_connections: u32,
    acquire_timeout: Duration,
    statement_timeout: Duration,
) -> Result<PgPool, sqlx::Error> {
    let options = PgConnectOptions::from_str(database_url)?.options([
        (
            "statement_timeout",
            format!("{}ms", statement_timeout.as_millis()),
        ),
        ("application_name", env!("CARGO_PKG_NAME").to_owned()),
    ]);

    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(acquire_timeout)
        .connect_with(options)
        .await
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

pub async fn validate_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    let rows =
        sqlx::query("select version, checksum, success from _sqlx_migrations order by version")
            .fetch_all(pool)
            .await?;
    let mut applied = std::collections::HashMap::new();

    for row in rows {
        let version: i64 = row.try_get("version")?;
        let checksum: Vec<u8> = row.try_get("checksum")?;
        let success: bool = row.try_get("success")?;
        if !success {
            return Err(MigrateError::Dirty(version));
        }
        applied.insert(version, checksum);
    }

    for migration in MIGRATOR
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
    {
        let Some(checksum) = applied.remove(&migration.version) else {
            return Err(MigrateError::VersionNotPresent(migration.version));
        };
        if checksum != migration.checksum.as_ref() {
            return Err(MigrateError::VersionMismatch(migration.version));
        }
    }

    if let Some(version) = applied.into_keys().next() {
        return Err(MigrateError::VersionMissing(version));
    }

    Ok(())
}
