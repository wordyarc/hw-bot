use sqlx::PgPool;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};

const POSTGRES_PORT: u16 = 5432;

#[tokio::test]
async fn migrations_apply_to_empty_postgres() -> anyhow::Result<()> {
    let postgres = GenericImage::new("postgres", "18.4-bookworm")
        .with_exposed_port(POSTGRES_PORT.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "hw_bot_test")
        .start()
        .await?;

    let host = postgres.get_host().await?;
    let port = postgres.get_host_port_ipv4(POSTGRES_PORT.tcp()).await?;
    let database_url = format!("postgres://postgres:postgres@{host}:{port}/hw_bot_test");
    let pool = PgPool::connect(&database_url).await?;

    hw_bot::db::run_migrations(&pool).await?;

    for table_name in [
        "student_groups",
        "subjects",
        "users",
        "teaching_courses",
        "teaching_course_teachers",
        "assignments",
        "submissions",
        "student_assignment_progress",
    ] {
        assert!(
            table_exists(&pool, table_name).await?,
            "missing table {table_name}"
        );
    }

    for index_name in [
        "submissions_one_pending_per_student_assignment_idx",
        "submissions_one_accepted_per_student_assignment_idx",
        "submissions_pending_review_queue_idx",
    ] {
        assert!(
            index_exists(&pool, index_name).await?,
            "missing index {index_name}"
        );
    }

    Ok(())
}

async fn table_exists(pool: &PgPool, table_name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        select exists (
            select 1
            from information_schema.tables
            where table_schema = 'public'
              and table_name = $1
        )
        "#,
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
}

async fn index_exists(pool: &PgPool, index_name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        select exists (
            select 1
            from pg_indexes
            where schemaname = 'public'
              and indexname = $1
        )
        "#,
    )
    .bind(index_name)
    .fetch_one(pool)
    .await
}
