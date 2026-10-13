use std::time::Duration;

use futures::future::join_all;
use hw_bot::db;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, ReuseDirective};
use tokio::sync::OnceCell;
use uuid::Uuid;

const POSTGRES_PORT: u16 = 5432;

/// Именованный контейнер, который переиспользуется между прогонами. Имя обязательно: по нему
/// testcontainers находит уже запущенный контейнер вместо того, чтобы поднимать новый.
const CONTAINER_NAME: &str = "hw-bot-test-postgres";
const MIGRATED_TEMPLATE: &str = "template_hw_bot";
const EMPTY_TEMPLATE: &str = "template1";
const DATABASE_LIFECYCLE_LOCK: i64 = 0x6877_626f_745f_6462;
const CLEANUP_CONNECTIONS: usize = 8;

/// Контейнер один на весь тестовый бинарник, база - на каждый тест.
static POSTGRES: OnceCell<Postgres> = OnceCell::const_new();

struct Postgres {
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

impl Postgres {
    fn url(&self, database: &str) -> String {
        format!("postgres://postgres@{}:{}/{database}", self.host, self.port)
    }
}

async fn postgres() -> &'static Postgres {
    POSTGRES
        .get_or_init(|| async {
            let container = GenericImage::new("postgres", "18.4-bookworm")
                .with_exposed_port(POSTGRES_PORT.tcp())
                .with_wait_for(WaitFor::message_on_stderr(
                    "database system is ready to accept connections",
                ))
                .with_env_var("POSTGRES_USER", "postgres")
                .with_env_var("POSTGRES_HOST_AUTH_METHOD", "trust")
                // Контейнер живёт в `static`, а статики не дропаются: без явного реюза каждый
                // прогон оставлял бы за собой ещё один Postgres. С реюзом он ровно один и
                // переживает прогон намеренно.
                .with_container_name(CONTAINER_NAME)
                .with_reuse(ReuseDirective::Always)
                // База одноразовая, долговечность не нужна. Без этого `create database` на
                // каждый тест гонит копию шаблона через WAL с fsync.
                .with_cmd([
                    "postgres",
                    "-c",
                    "fsync=off",
                    "-c",
                    "full_page_writes=off",
                    "-c",
                    "synchronous_commit=off",
                    "-c",
                    "max_connections=200",
                ])
                .start()
                .await
                .expect("postgres container starts (is docker running?)");

            let host = container
                .get_host()
                .await
                .expect("container host")
                .to_string();
            let port = container
                .get_host_port_ipv4(POSTGRES_PORT.tcp())
                .await
                .expect("container port");

            let postgres = Postgres {
                _container: container,
                host,
                port,
            };
            let mut admin = connect_when_ready(&postgres.url("postgres")).await;
            lock_database_lifecycle(&mut admin).await;
            drop_stale_databases(&postgres, &mut admin).await;
            ensure_migrated_template(&postgres, &mut admin).await;
            admin.close().await.expect("close readiness connection");

            postgres
        })
        .await
}

/// Контейнер переиспользуется, поэтому базы прошлых прогонов убираем сами - иначе они копятся.
/// Ошибки глотаем: занятую параллельным прогоном базу дропнуть нельзя, и это нормально.
async fn drop_stale_databases(postgres: &Postgres, admin: &mut PgConnection) {
    let stale: Vec<String> = sqlx::query_scalar(
        r"select datname from pg_database
          where datname like 'test\_%'
            and not exists (
                select 1 from pg_stat_activity where pg_stat_activity.datid = pg_database.oid
            )",
    )
    .fetch_all(&mut *admin)
    .await
    .unwrap_or_default();

    join_all((0..CLEANUP_CONNECTIONS).map(|worker| {
        let names = stale.iter().skip(worker).step_by(CLEANUP_CONNECTIONS);
        async move {
            let Ok(mut connection) = PgConnection::connect(&postgres.url("postgres")).await else {
                return;
            };
            for name in names {
                let _ = sqlx::query(AssertSqlSafe(format!(
                    r#"drop database if exists "{name}""#
                )))
                .execute(&mut connection)
                .await;
            }
            let _ = connection.close().await;
        }
    }))
    .await;
}

async fn ensure_migrated_template(postgres: &Postgres, admin: &mut PgConnection) {
    let exists: bool =
        sqlx::query_scalar("select exists (select 1 from pg_database where datname = $1)")
            .bind(MIGRATED_TEMPLATE)
            .fetch_one(&mut *admin)
            .await
            .expect("look up the migrated template");

    if exists {
        let pool = connect_template(postgres).await;
        let current = db::validate_migrations(&pool).await.is_ok();
        pool.close().await;
        if current {
            return;
        }
        sqlx::query(AssertSqlSafe(format!(
            r#"drop database "{MIGRATED_TEMPLATE}""#
        )))
        .execute(&mut *admin)
        .await
        .expect("drop the outdated template");
    }

    sqlx::query(AssertSqlSafe(format!(
        r#"create database "{MIGRATED_TEMPLATE}""#
    )))
    .execute(&mut *admin)
    .await
    .expect("create the migrated template");
    let pool = connect_template(postgres).await;
    db::run_migrations(&pool)
        .await
        .expect("migrations apply to the template");
    pool.close().await;
}

async fn connect_template(postgres: &Postgres) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .connect(&postgres.url(MIGRATED_TEMPLATE))
        .await
        .expect("connect to the migrated template")
}

async fn lock_database_lifecycle(conn: &mut PgConnection) {
    sqlx::query("select pg_advisory_lock($1)")
        .bind(DATABASE_LIFECYCLE_LOCK)
        .execute(conn)
        .await
        .expect("lock test database lifecycle");
}

async fn share_database_lifecycle(conn: &mut PgConnection) {
    sqlx::query("select pg_advisory_lock_shared($1)")
        .bind(DATABASE_LIFECYCLE_LOCK)
        .execute(conn)
        .await
        .expect("share test database lifecycle");
}

// Первое ready относится к временному серверу initdb. Первый admin-connect обязан дождаться
// SQL через опубликованный TCP-порт, прежде чем создавать или удалять тестовые базы.
async fn connect_when_ready(url: &str) -> PgConnection {
    let mut last_error = None;
    let result = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let attempt = async {
                let mut connection = PgConnection::connect(url).await?;
                sqlx::query("select 1").execute(&mut connection).await?;
                Ok::<_, sqlx::Error>(connection)
            }
            .await;
            match attempt {
                Ok(connection) => return connection,
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    result.unwrap_or_else(|_| panic!("PostgreSQL did not become SQL-ready: {last_error:?}"))
}

pub async fn fresh_database() -> PgPool {
    connect_fresh(PgPoolOptions::new().max_connections(4), EMPTY_TEMPLATE).await
}

pub async fn saturated_database() -> PgPool {
    connect_fresh(
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(250)),
        MIGRATED_TEMPLATE,
    )
    .await
}

async fn connect_fresh(options: PgPoolOptions, template: &str) -> PgPool {
    let postgres = postgres().await;
    let name = format!("test_{}", Uuid::now_v7().simple());

    // Долгоживущий admin-пул в `static` не годится: каждый `#[tokio::test]` крутится на своём
    // runtime, а сокеты пула привязаны к реактору того теста, который его создал, и умирают
    // вместе с ним. Поэтому коннект одноразовый и открывается на текущем runtime.
    // Имя базы нельзя передать bind-параметром, но оно только что сгенерировано из uuid.
    let mut admin = PgConnection::connect(&postgres.url("postgres"))
        .await
        .expect("connect to the maintenance database");
    share_database_lifecycle(&mut admin).await;

    sqlx::query(AssertSqlSafe(format!(
        r#"create database "{name}" template "{template}""#
    )))
    .execute(&mut admin)
    .await
    .expect("create a database for the test");

    // Lock держим до первого подключения: другой прогон уже не сможет удалить только что
    // созданную базу в зазоре между `create database` и коннектом.
    let pool = options
        .connect(&postgres.url(&name))
        .await
        .expect("connect to the test database");
    admin
        .close()
        .await
        .expect("close the maintenance connection");

    pool
}

pub async fn migrated_database() -> PgPool {
    connect_fresh(PgPoolOptions::new().max_connections(4), MIGRATED_TEMPLATE).await
}

pub async fn wait_for_blocked_connection(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "select exists (
                    select 1 from pg_stat_activity
                    where datid = (select oid from pg_database where datname = current_database())
                      and $1 = any(pg_blocking_pids(pid))
                )",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await
            .expect("read blocked connections");
            if waiting {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no connection in this database is blocked by backend {blocker}"));
}
