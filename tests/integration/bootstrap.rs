use std::path::Path;
use std::time::Duration;

use hw_bot::db::{self, BootstrapOutcome};
use hw_bot::test_support::bootstrap_seed as seed;
use serde_json::json;
use sqlx::PgPool;

use crate::harness::{migrated_database, wait_for_blocked_connection};

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
        join teaching_course_groups
          on teaching_course_groups.teaching_course_id = teaching_courses.id
        join student_groups on student_groups.id = teaching_course_groups.student_group_id
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
async fn bootstrap_gives_every_course_an_invite() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    db::run_bootstrap(&pool, &seed()).await?;

    let generation: i64 = sqlx::query_scalar(
        "select generation from teaching_course_invites
         join teaching_courses on teaching_courses.id = teaching_course_invites.teaching_course_id
         join subjects on subjects.id = teaching_courses.subject_id
         where subjects.name = 'Backend Development'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(generation, 1);
    Ok(())
}

fn seed_with_groups(codes: &[&str]) -> serde_json::Value {
    let mut seed = seed();
    seed["student_groups"] = codes
        .iter()
        .map(|code| json!({"code": code, "education_level": "bachelor", "study_year": 1}))
        .collect();
    seed["courses"][0]["student_group_codes"] = json!(codes);
    seed
}

#[tokio::test]
async fn a_course_of_five_groups_is_one_course_with_one_invite() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    db::run_bootstrap(&pool, &seed_with_groups(&["g1", "g2", "g3", "g4", "g5"])).await?;

    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    assert_eq!(count(&pool, "teaching_course_groups").await?, 5);
    assert_eq!(count(&pool, "teaching_course_invites").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_repeated_bootstrap_changes_nothing() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let seed = seed();
    assert_eq!(
        db::run_bootstrap(&pool, &seed).await?,
        BootstrapOutcome::Applied
    );
    sqlx::query("update teaching_course_invites set generation = 7")
        .execute(&pool)
        .await?;

    let outcome = db::run_bootstrap(&pool, &seed).await?;

    assert_eq!(outcome, BootstrapOutcome::AlreadyPopulated);
    assert_eq!(count(&pool, "student_groups").await?, 1);
    assert_eq!(count(&pool, "subjects").await?, 1);
    assert_eq!(count(&pool, "users").await?, 2);
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    let generation: i64 = sqlx::query_scalar("select generation from teaching_course_invites")
        .fetch_one(&pool)
        .await?;
    assert_eq!(generation, 7);
    Ok(())
}

#[tokio::test]
async fn a_group_renamed_in_the_bot_is_not_created_again() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let seed = seed();
    db::run_bootstrap(&pool, &seed).await?;
    sqlx::query("update student_groups set code = 'renamed'")
        .execute(&pool)
        .await?;

    db::run_bootstrap(&pool, &seed).await?;

    let codes: Vec<String> = sqlx::query_scalar("select code from student_groups")
        .fetch_all(&pool)
        .await?;
    assert_eq!(codes, vec!["renamed".to_owned()]);
    Ok(())
}

#[tokio::test]
async fn a_missing_file_on_a_populated_database_is_skipped() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    db::run_bootstrap(&pool, &seed()).await?;

    let outcome = db::run_bootstrap_file(&pool, Path::new("/nonexistent/bootstrap.json")).await?;

    assert_eq!(outcome, BootstrapOutcome::AlreadyPopulated);
    Ok(())
}

#[tokio::test]
async fn a_broken_file_on_a_populated_database_is_skipped() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    db::run_bootstrap(&pool, &seed()).await?;
    let not_json = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"));

    let outcome = db::run_bootstrap_file(&pool, not_json).await?;

    assert_eq!(outcome, BootstrapOutcome::AlreadyPopulated);
    assert_eq!(count(&pool, "users").await?, 2);
    Ok(())
}

#[tokio::test]
async fn a_missing_file_on_an_empty_database_is_an_error() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    let error = db::run_bootstrap_file(&pool, Path::new("/nonexistent/bootstrap.json"))
        .await
        .unwrap_err();

    assert!(matches!(error, db::BootstrapError::File { .. }), "{error}");
    Ok(())
}

#[tokio::test]
async fn the_file_of_an_empty_database_is_applied() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let file = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/config/bootstrap.json"
    ));

    let outcome = db::run_bootstrap_file(&pool, file).await?;

    assert_eq!(outcome, BootstrapOutcome::Applied);
    assert!(count(&pool, "users").await? > 0);
    Ok(())
}

#[tokio::test]
async fn bootstrap_creates_a_sandbox_subject() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut seed = seed();
    seed["subjects"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name": "Sandbox", "sandbox": true}));

    db::run_bootstrap(&pool, &seed).await?;

    assert!(is_sandbox(&pool, "Sandbox").await?);
    Ok(())
}

#[tokio::test]
async fn a_subject_without_the_sandbox_field_is_regular() -> anyhow::Result<()> {
    let pool = migrated_database().await;

    db::run_bootstrap(&pool, &seed()).await?;

    assert!(!is_sandbox(&pool, "Backend Development").await?);
    Ok(())
}

#[tokio::test]
async fn a_failure_on_an_empty_database_rolls_back_the_whole_seed() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    sqlx::query("insert into subjects (name) values ('Backend Development')")
        .execute(&pool)
        .await?;

    db::run_bootstrap(&pool, &seed()).await.unwrap_err();

    assert_eq!(count(&pool, "users").await?, 0);
    assert_eq!(count(&pool, "student_groups").await?, 0);
    assert_eq!(count(&pool, "teaching_courses").await?, 0);
    Ok(())
}

#[tokio::test]
async fn a_seed_with_two_courses_of_one_group_and_subject_is_rejected() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut conflicting = seed_with_groups(&["g1"]);
    let course = conflicting["courses"][0].clone();
    conflicting["courses"].as_array_mut().unwrap().push(course);

    let error = db::run_bootstrap(&pool, &conflicting).await.unwrap_err();

    assert!(
        matches!(&error, db::BootstrapError::InvalidData(_)),
        "{error}"
    );
    assert_eq!(count(&pool, "users").await?, 0);
    Ok(())
}

#[tokio::test]
async fn the_second_of_two_parallel_bootstraps_changes_nothing() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut lock_connection = pool.acquire().await?;
    let blocker = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *lock_connection)
        .await?;
    sqlx::query("select pg_advisory_lock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;

    let (first_pool, second_pool) = (pool.clone(), pool.clone());
    let first = tokio::spawn(async move { db::run_bootstrap(&first_pool, &seed()).await });
    let second = tokio::spawn(async move { db::run_bootstrap(&second_pool, &seed()).await });
    wait_for_blocked_connection(&pool, blocker).await;

    sqlx::query("select pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;
    let timeout = Duration::from_secs(5);
    let first = tokio::time::timeout(timeout, first).await???;
    let second = tokio::time::timeout(timeout, second).await???;

    let mut outcomes = [first, second].map(|outcome| outcome == BootstrapOutcome::Applied);
    outcomes.sort_unstable();
    assert_eq!(outcomes, [false, true]);
    assert_eq!(count(&pool, "users").await?, 2);
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    Ok(())
}

#[tokio::test]
async fn bootstrap_waits_for_its_advisory_lock() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut lock_connection = pool.acquire().await?;
    let blocker = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *lock_connection)
        .await?;
    sqlx::query("select pg_advisory_lock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;

    let task_pool = pool.clone();
    let bootstrap = tokio::spawn(async move { db::run_bootstrap(&task_pool, &seed()).await });

    wait_for_blocked_connection(&pool, blocker).await;
    assert!(!bootstrap.is_finished());

    sqlx::query("select pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(BOOTSTRAP_LOCK)
        .execute(&mut *lock_connection)
        .await?;
    tokio::time::timeout(Duration::from_secs(5), bootstrap)
        .await
        .expect("bootstrap continues after its advisory lock is released")??;
    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    Ok(())
}

async fn is_sandbox(pool: &PgPool, subject: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("select sandbox from subjects where name = $1")
        .bind(subject)
        .fetch_one(pool)
        .await
}

async fn count(pool: &PgPool, table: &str) -> Result<i64, sqlx::Error> {
    let query = match table {
        "student_groups" => "select count(*) from student_groups",
        "subjects" => "select count(*) from subjects",
        "users" => "select count(*) from users",
        "teaching_courses" => "select count(*) from teaching_courses",
        "teaching_course_groups" => "select count(*) from teaching_course_groups",
        "teaching_course_invites" => "select count(*) from teaching_course_invites",
        "teaching_course_teachers" => "select count(*) from teaching_course_teachers",
        _ => unreachable!("unsupported table"),
    };

    sqlx::query_scalar(query).fetch_one(pool).await
}
