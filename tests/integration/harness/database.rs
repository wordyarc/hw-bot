use std::time::Duration;

use futures::future::join_all;
use hw_bot::db;
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, ReuseDirective};
use tokio::sync::OnceCell;
use tokio::time::Instant;
use uuid::Uuid;

const POSTGRES_PORT: u16 = 5432;

/// Именованный контейнер, который переиспользуется между прогонами. Имя обязательно: по нему
/// testcontainers находит уже запущенный контейнер вместо того, чтобы поднимать новый.
const CONTAINER_NAME: &str = "hw-bot-test-postgres";
const MIGRATED_TEMPLATE: &str = "template_hw_bot";
const EMPTY_TEMPLATE: &str = "template1";
const DATABASE_LIFECYCLE_LOCK: i64 = 0x6877_626f_745f_6462;
const CLEANUP_CONNECTIONS: usize = 8;
const CONNECT_DEADLINE: Duration = Duration::from_secs(10);
const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(50);
const FIRST_CONNECTION_POLL: Duration = Duration::from_millis(5);
const WARM_CONNECTIONS: u32 = 2;

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
                // База одноразовая: ни долговечность, ни автовакуум ей не нужны.
                .with_cmd([
                    "postgres",
                    "-c",
                    "fsync=off",
                    "-c",
                    "full_page_writes=off",
                    "-c",
                    "synchronous_commit=off",
                    "-c",
                    "autovacuum=off",
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
            let Ok(mut connection) = connect(&postgres.url("postgres")).await else {
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
    let url = postgres.url(MIGRATED_TEMPLATE);
    retry_transient(|| PgPoolOptions::new().max_connections(1).connect(&url))
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
            .test_before_acquire(false)
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
    let mut admin = connect(&postgres.url("postgres"))
        .await
        .expect("connect to the maintenance database");
    share_database_lifecycle(&mut admin).await;

    sqlx::query(AssertSqlSafe(format!(
        r#"create database "{name}" template "{template}" strategy file_copy"#
    )))
    .execute(&mut admin)
    .await
    .expect("create a database for the test");

    // Lock держим до первого подключения: другой прогон уже не сможет удалить только что
    // созданную базу в зазоре между `create database` и коннектом.
    let warm = options.get_max_connections().min(WARM_CONNECTIONS);
    let pool = open_warm(
        options
            .min_connections(warm)
            .idle_timeout(None)
            .max_lifetime(None),
        &postgres.url(&name),
    )
    .await;
    admin
        .close()
        .await
        .expect("close the maintenance connection");

    pool
}

async fn open_warm(options: PgPoolOptions, url: &str) -> PgPool {
    let warm = options.get_min_connections();
    let open = || {
        options
            .clone()
            .connect_lazy(url)
            .expect("valid test database url")
    };
    let deadline = Instant::now() + CONNECT_DEADLINE;
    let mut pool = open();
    while Instant::now() < deadline {
        tokio::time::sleep(FIRST_CONNECTION_POLL).await;
        if pool.num_idle() >= warm as usize {
            return pool;
        }
        if pool.size() < warm && pool.num_idle() == pool.size() as usize {
            tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            pool = open();
        }
    }

    let cause = match tokio::time::timeout(CONNECT_DEADLINE, PgConnection::connect(url)).await {
        Ok(Ok(connection)) => {
            let _ = connection.close().await;
            "the server accepts connections but the pool did not open them".to_owned()
        }
        Ok(Err(error)) => format!("{error:?}"),
        Err(_) => "a direct connection does not open either".to_owned(),
    };
    panic!("the pool opens its warm connections to the test database: {cause}");
}

async fn connect(url: &str) -> Result<PgConnection, sqlx::Error> {
    retry_transient(|| PgConnection::connect(url)).await
}

async fn retry_transient<T, F>(mut attempt: impl FnMut() -> F) -> Result<T, sqlx::Error>
where
    F: Future<Output = Result<T, sqlx::Error>>,
{
    let deadline = Instant::now() + CONNECT_DEADLINE;
    loop {
        match tokio::time::timeout_at(deadline, attempt()).await {
            Ok(Err(error))
                if is_transient(&error) && Instant::now() + CONNECT_RETRY_DELAY < deadline =>
            {
                tokio::time::sleep(CONNECT_RETRY_DELAY).await;
            }
            Ok(result) => return result,
            Err(elapsed) => return Err(sqlx::Error::Io(elapsed.into())),
        }
    }
}

fn is_transient(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Io(_) | sqlx::Error::Protocol(_))
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
