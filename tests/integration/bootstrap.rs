use std::time::Duration;

use hw_bot::db;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::harness::migrated_database;

const BOOTSTRAP_LOCK: &str = "hw-bot/bootstrap/v1";

#[tokio::test]
async fn bootstrap_creates_complete_initial_structure() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    db::run_bootstrap(&pool, &seed()).await?;

    assert_eq!(count(&pool, "student_groups").await?, 1);
    assert_eq!(count(&pool, "subjects").await?, 1);
    assert_eq!(count(&pool, "users").await?, 2);
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    assert_eq!(count(&pool, "teaching_course_teachers").await?, 1);

    let rows: Vec<(String, String, i64, String)> = sqlx::query_as(
        r#"
        select subjects.name, student_groups.code, users.telegram_user_id, users.role
        from teaching_courses
        join subjects on subjects.id = teaching_courses.subject_id
        join student_groups on student_groups.id = teaching_courses.student_group_id
        join teaching_course_teachers
          on teaching_course_teachers.teaching_course_id = teaching_courses.id
        join users on users.id = teaching_course_teachers.teacher_id
        "#,
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(
        rows,
        vec![(
            "Backend Development".to_owned(),
            "b9122-02.03.21".to_owned(),
            100_000_002,
            "teacher".to_owned(),
        )]
    );
    Ok(())
}

#[tokio::test]
async fn bootstrap_can_be_repeated_without_duplicates() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let seed = seed();

    db::run_bootstrap(&pool, &seed).await?;
    db::run_bootstrap(&pool, &seed).await?;

    assert_eq!(count(&pool, "student_groups").await?, 1);
    assert_eq!(count(&pool, "subjects").await?, 1);
    assert_eq!(count(&pool, "users").await?, 2);
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    assert_eq!(count(&pool, "teaching_course_teachers").await?, 1);
    Ok(())
}

#[tokio::test]
async fn conflicting_existing_staff_aborts_without_partial_seed() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    sqlx::query(
        r#"
        with seed_group as (
            insert into student_groups (code, education_level, study_year)
            values ('existing', 'bachelor', 1)
            returning id
        )
        insert into users (telegram_user_id, role, display_name, student_group_id)
        select 100000002, 'student', 'Existing Student', id
        from seed_group
        "#,
    )
    .execute(&pool)
    .await?;

    let error = db::run_bootstrap(&pool, &seed()).await.unwrap_err();

    assert!(error.to_string().contains("bootstrap:"), "{error}");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "select count(*) from student_groups where code = 'b9122-02.03.21'"
        )
        .fetch_one(&pool)
        .await?,
        0
    );
    assert_eq!(count(&pool, "subjects").await?, 0);
    assert_eq!(count(&pool, "teaching_courses").await?, 0);
    Ok(())
}

#[tokio::test]
async fn bootstrap_rejects_text_that_is_empty_after_domain_trimming() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut seed = seed();
    seed["admin"]["display_name"] = json!("\u{a0}");

    let error = db::run_bootstrap(&pool, &seed).await.unwrap_err();

    assert!(error.to_string().contains("bootstrap:"), "{error}");
    assert_eq!(count(&pool, "users").await?, 0);
    assert_eq!(count(&pool, "student_groups").await?, 0);
    Ok(())
}

#[tokio::test]
async fn bootstrap_rejects_a_manually_seeded_course_without_teacher() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    sqlx::query(
        r#"
        with seed_group as (
            insert into student_groups (code, education_level, study_year)
            values ('manual-group', 'master', 1)
            returning id
        ), seed_subject as (
            insert into subjects (name)
            values ('Manual Subject')
            returning id
        )
        insert into teaching_courses (subject_id, student_group_id)
        select seed_subject.id, seed_group.id
        from seed_subject, seed_group
        "#,
    )
    .execute(&pool)
    .await?;

    let error = db::run_bootstrap(&pool, &seed()).await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("every teaching course must have at least one teacher"),
        "{error}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "select count(*) from subjects where name = 'Backend Development'"
        )
        .fetch_one(&pool)
        .await?,
        0
    );
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    Ok(())
}

#[tokio::test]
async fn bootstrap_waits_for_its_advisory_lock() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut lock_connection = pool.acquire().await?;
    sqlx::query("select pg_advisory_lock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;

    let task_pool = pool.clone();
    let bootstrap = tokio::spawn(async move { db::run_bootstrap(&task_pool, &seed()).await });

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let waiting = sqlx::query_scalar::<_, bool>(
                "select exists (select 1 from pg_locks where locktype = 'advisory' and not granted)",
            )
            .fetch_one(&pool)
            .await
            .expect("read advisory locks");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bootstrap did not wait for the advisory lock");
    assert!(!bootstrap.is_finished());

    sqlx::query("select pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;
    bootstrap.await??;
    Ok(())
}

fn seed() -> Value {
    json!({
        "admin": {
            "telegram_user_id": 100000001,
            "display_name": "First Admin"
        },
        "teachers": [{
            "telegram_user_id": 100000002,
            "display_name": "Course Teacher"
        }],
        "student_groups": [{
            "code": "b9122-02.03.21",
            "education_level": "bachelor",
            "study_year": 1
        }],
        "subjects": [{
            "name": "Backend Development"
        }],
        "courses": [{
            "subject_name": "Backend Development",
            "student_group_code": "b9122-02.03.21",
            "teacher_telegram_user_ids": [100000002]
        }]
    })
}

async fn count(pool: &PgPool, table: &str) -> Result<i64, sqlx::Error> {
    let query = match table {
        "student_groups" => "select count(*) from student_groups",
        "subjects" => "select count(*) from subjects",
        "users" => "select count(*) from users",
        "teaching_courses" => "select count(*) from teaching_courses",
        "teaching_course_teachers" => "select count(*) from teaching_course_teachers",
        _ => unreachable!("unsupported table"),
    };

    sqlx::query_scalar(query).fetch_one(pool).await
}
