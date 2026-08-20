use std::time::Duration;

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
const DATABASE_LIFECYCLE_LOCK: i64 = 0x6877_626f_745f_6462;

/// Контейнер один на весь тестовый бинарник, база - на каждый тест.
static POSTGRES: OnceCell<Postgres> = OnceCell::const_new();

struct Postgres {
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

impl Postgres {
    fn url(&self, database: &str) -> String {
        format!(
            "postgres://postgres:postgres@{}:{}/{database}",
            self.host, self.port
        )
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
                .with_env_var("POSTGRES_PASSWORD", "postgres")
                // Контейнер живёт в `static`, а статики не дропаются: без явного реюза каждый
                // прогон оставлял бы за собой ещё один Postgres. С реюзом он ровно один и
                // переживает прогон намеренно.
                .with_container_name(CONTAINER_NAME)
                .with_reuse(ReuseDirective::Always)
                // База одноразовая, долговечность не нужна. Без этого `create database` на
                // каждый тест гонит копию template1 через WAL с fsync.
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
            drop_stale_databases(&postgres.url("postgres")).await;

            postgres
        })
        .await
}

/// Контейнер переиспользуется, поэтому базы прошлых прогонов убираем сами - иначе они копятся.
/// Ошибки глотаем: занятую параллельным прогоном базу дропнуть нельзя, и это нормально.
async fn drop_stale_databases(url: &str) {
    let Ok(mut conn) = PgConnection::connect(url).await else {
        return;
    };
    lock_database_lifecycle(&mut conn).await;

    let stale: Vec<String> =
        sqlx::query_scalar(r"select datname from pg_database where datname like 'test\_%'")
            .fetch_all(&mut conn)
            .await
            .unwrap_or_default();

    for name in stale {
        let _ = sqlx::query(AssertSqlSafe(format!(
            r#"drop database if exists "{name}""#
        )))
        .execute(&mut conn)
        .await;
    }

    let _ = conn.close().await;
}

async fn lock_database_lifecycle(conn: &mut PgConnection) {
    sqlx::query("select pg_advisory_lock($1)")
        .bind(DATABASE_LIFECYCLE_LOCK)
        .execute(conn)
        .await
        .expect("lock test database lifecycle");
}

/// Образ печатает ready ещё на этапе initdb, поэтому первый коннект иногда упирается в закрытый
/// порт. Дешевле переподключиться, чем ловить редкий флак.
async fn connect_with_retry(options: PgPoolOptions, url: &str) -> PgPool {
    let mut last_error = None;

    for attempt in 0..10 {
        match options.clone().connect(url).await {
            Ok(pool) => return pool,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt + 1))).await;
            }
        }
    }

    panic!("cannot connect to {url}: {}", last_error.expect("an error"));
}

pub async fn fresh_database() -> PgPool {
    connect_fresh(PgPoolOptions::new().max_connections(4)).await
}

pub async fn saturated_database() -> PgPool {
    let pool = connect_fresh(
        PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(250)),
    )
    .await;
    db::run_migrations(&pool).await.expect("migrations apply");

    pool
}

async fn connect_fresh(options: PgPoolOptions) -> PgPool {
    let postgres = postgres().await;
    let name = format!("test_{}", Uuid::now_v7().simple());

    // Долгоживущий admin-пул в `static` не годится: каждый `#[tokio::test]` крутится на своём
    // runtime, а сокеты пула привязаны к реактору того теста, который его создал, и умирают
    // вместе с ним. Поэтому коннект одноразовый и открывается на текущем runtime.
    // Имя базы нельзя передать bind-параметром, но оно только что сгенерировано из uuid.
    let mut admin = PgConnection::connect(&postgres.url("postgres"))
        .await
        .expect("connect to the maintenance database");
    lock_database_lifecycle(&mut admin).await;

    sqlx::query(AssertSqlSafe(format!(r#"create database "{name}""#)))
        .execute(&mut admin)
        .await
        .expect("create a database for the test");

    // Lock держим до первого подключения: другой прогон уже не сможет удалить только что
    // созданную базу в зазоре между `create database` и коннектом.
    let pool = connect_with_retry(options, &postgres.url(&name)).await;
    admin
        .close()
        .await
        .expect("close the maintenance connection");

    pool
}

pub async fn migrated_database() -> PgPool {
    let pool = fresh_database().await;
    db::run_migrations(&pool).await.expect("migrations apply");

    pool
}
