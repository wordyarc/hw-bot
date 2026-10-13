use std::time::Duration;

use hw_bot::db;
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
async fn a_group_added_to_a_stored_course_keeps_its_invite_and_assignments() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    db::run_bootstrap(&pool, &seed_with_groups(&["g1"])).await?;
    sqlx::query("update teaching_course_invites set generation = 7")
        .execute(&pool)
        .await?;
    sqlx::query(
        r#"
        with issued as (
            insert into assignments (
                teaching_course_id, subject_id, title, body, max_attempts, status, created_by,
                deadline_at
            )
            select teaching_courses.id, teaching_courses.subject_id, 'Lab', 'Body', 1, 'draft',
                   users.id, now() + interval '7 days'
            from teaching_courses, users
            where users.telegram_user_id = 100000002
            returning id, teaching_course_id
        )
        insert into assignment_groups (assignment_id, teaching_course_id, student_group_id)
        select issued.id, issued.teaching_course_id, student_groups.id
        from issued, student_groups
        "#,
    )
    .execute(&pool)
    .await?;

    db::run_bootstrap(&pool, &seed_with_groups(&["g1", "g2"])).await?;

    assert_eq!(count(&pool, "teaching_courses").await?, 1);
    assert_eq!(count(&pool, "teaching_course_groups").await?, 2);
    let generation: i64 = sqlx::query_scalar("select generation from teaching_course_invites")
        .fetch_one(&pool)
        .await?;
    assert_eq!(generation, 7);
    let issued: Vec<String> = sqlx::query_scalar(
        "select student_groups.code from assignment_groups
         join student_groups on student_groups.id = assignment_groups.student_group_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(issued, vec!["g1".to_owned()]);
    Ok(())
}

#[tokio::test]
async fn a_seed_joining_two_stored_courses_of_a_subject_is_rolled_back() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let mut split = seed_with_groups(&["g1", "g2"]);
    split["courses"] = json!([
        {
            "subject_name": "Backend Development",
            "student_group_codes": ["g1"],
            "teacher_telegram_user_ids": [100000002]
        },
        {
            "subject_name": "Backend Development",
            "student_group_codes": ["g2"],
            "teacher_telegram_user_ids": [100000002]
        }
    ]);
    db::run_bootstrap(&pool, &split).await?;
    let mut joined = seed_with_groups(&["g1", "g2", "g3"]);
    joined["courses"][0]["student_group_codes"] = json!(["g1", "g2"]);

    let error = db::run_bootstrap(&pool, &joined).await.unwrap_err();

    assert!(
        matches!(&error, db::BootstrapError::InvalidData(_)),
        "{error}"
    );
    assert_eq!(count(&pool, "teaching_courses").await?, 2);
    assert_eq!(count(&pool, "student_groups").await?, 2);
    Ok(())
}

#[tokio::test]
async fn a_seed_splitting_a_stored_course_is_rolled_back() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    db::run_bootstrap(&pool, &seed_with_groups(&["g1", "g2"])).await?;
    let mut split = seed_with_groups(&["g1", "g2"]);
    split["courses"] = json!([
        {
            "subject_name": "Backend Development",
            "student_group_codes": ["g1"],
            "teacher_telegram_user_ids": [100000002]
        },
        {
            "subject_name": "Backend Development",
            "student_group_codes": ["g2"],
            "teacher_telegram_user_ids": [100000001]
        }
    ]);

    let error = db::run_bootstrap(&pool, &split).await.unwrap_err();

    assert!(
        matches!(&error, db::BootstrapError::InvalidData(_)),
        "{error}"
    );
    assert_eq!(count(&pool, "teaching_course_teachers").await?, 1);
    Ok(())
}

#[tokio::test]
async fn a_repeated_bootstrap_never_resets_a_rotated_invite() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    let seed = seed();
    db::run_bootstrap(&pool, &seed).await?;
    sqlx::query("update teaching_course_invites set generation = 7")
        .execute(&pool)
        .await?;

    db::run_bootstrap(&pool, &seed).await?;

    let generation: i64 = sqlx::query_scalar("select generation from teaching_course_invites")
        .fetch_one(&pool)
        .await?;

    assert_eq!(generation, 7);
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
async fn a_conflicting_sandbox_flag_aborts_the_whole_bootstrap() -> anyhow::Result<()> {
    let pool = migrated_database().await;
    db::run_bootstrap(&pool, &seed()).await?;
    let mut changed = seed();
    changed["subjects"][0]["sandbox"] = json!(true);
    changed["student_groups"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "code": "m9241-09.04.03",
            "education_level": "master",
            "study_year": 1
        }));

    let error = db::run_bootstrap(&pool, &changed).await.unwrap_err();

    assert!(
        matches!(&error, db::BootstrapError::InvalidData(message) if message.contains("Backend Development")),
        "{error}"
    );
    assert!(!is_sandbox(&pool, "Backend Development").await?);
    assert_eq!(count(&pool, "student_groups").await?, 1);
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

    assert!(
        matches!(&error, db::BootstrapError::InvalidData(message) if message.contains("100000002")),
        "{error}"
    );
    let users: Vec<i64> = sqlx::query_scalar("select telegram_user_id from users")
        .fetch_all(&pool)
        .await?;
    assert_eq!(
        users,
        vec![100_000_002],
        "the admin seeded before the conflict is rolled back"
    );
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
        ), seed_course as (
            insert into teaching_courses (subject_id)
            select id from seed_subject
            returning id, subject_id
        )
        insert into teaching_course_groups (teaching_course_id, subject_id, student_group_id)
        select seed_course.id, seed_course.subject_id, seed_group.id
        from seed_course, seed_group
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
