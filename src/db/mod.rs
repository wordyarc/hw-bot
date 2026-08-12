use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::postgres::PgPoolOptions;

pub mod academic;
pub mod assignments;
pub mod attempts;
mod bootstrap;
mod convert;
mod enums;
mod error;
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

pub async fn run_migrations(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}
