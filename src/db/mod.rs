use sqlx::PgPool;
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
pub mod notifications;
pub mod submissions;
pub mod users;

pub use bootstrap::{BootstrapError, run_bootstrap};
pub use error::{DbError, DbResult};

pub(crate) use convert::checked_num_cast;

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
