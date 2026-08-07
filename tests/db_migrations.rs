mod common;

use chrono::{Duration, Utc};
use common::{TestApp, postgres_pool};
use hw_bot::domain::{AssignmentStatus, EducationLevel, SubmissionStatus};
use sqlx::PgPool;

#[tokio::test]
async fn migrations_apply_to_empty_postgres() -> anyhow::Result<()> {
    let (_postgres, pool) = postgres_pool().await?;

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

/// Имена вариантов и лимиты курсов живут в двух местах: `str_enum!` в домене и CHECK-констрейнты
/// миграции.
#[tokio::test]
async fn enum_variants_and_study_year_limits_match_the_check_constraints() -> anyhow::Result<()> {
    let test_app = TestApp::start().await?;
    let (pool, fixture) = (&test_app.pool, &test_app.fixture);

    const INSERT_GROUP: &str = r#"
        insert into student_groups (code, education_level, study_year)
        values ($1, $2, $3)
    "#;

    for level in EducationLevel::ALL {
        let max = i16::from(level.max_study_year());

        sqlx::query(INSERT_GROUP)
            .bind(format!("max-{}", level.as_str()))
            .bind(level.as_str())
            .bind(max)
            .execute(pool)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "{} year {max} rejected by the schema: {error}",
                    level.as_str()
                )
            });

        let over_limit = sqlx::query(INSERT_GROUP)
            .bind(format!("over-{}", level.as_str()))
            .bind(level.as_str())
            .bind(max + 1)
            .execute(pool)
            .await;

        assert!(
            over_limit.is_err(),
            "the schema accepts {} year {}, but max_study_year() says {max}",
            level.as_str(),
            max + 1
        );
    }

    for status in AssignmentStatus::ALL {
        sqlx::query(
            r#"
            insert into assignments (
                teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
                status, created_by, deadline_at
            )
            values ($1, $2, $3, $4, 'body', 1, $5, $6, $7)
            "#,
        )
        .bind(fixture.course.id().as_uuid())
        .bind(fixture.subject.id().as_uuid())
        .bind(fixture.group.id().as_uuid())
        .bind(format!("assignment-{}", status.as_str()))
        .bind(status.as_str())
        .bind(fixture.teacher.id().as_uuid())
        .bind(Utc::now() + Duration::days(7))
        .execute(pool)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "assignment status {} rejected by the schema: {error}",
                status.as_str()
            )
        });
    }

    for (attempt, status) in SubmissionStatus::ALL.iter().enumerate() {
        let reviewed = *status != SubmissionStatus::PendingReview;

        sqlx::query(
            r#"
            insert into submissions (
                assignment_id, student_id, attempt_number, content, status,
                reviewed_by, reviewed_at, review_comment
            )
            values ($1, $2, $3, 'https://example.test/pr', $4, $5, $6, $7)
            "#,
        )
        .bind(fixture.assignment.id().as_uuid())
        .bind(fixture.student.id().as_uuid())
        .bind(attempt as i16 + 1)
        .bind(status.as_str())
        .bind(reviewed.then(|| fixture.teacher.id().as_uuid()))
        .bind(reviewed.then(Utc::now))
        .bind(reviewed.then_some("comment"))
        .execute(pool)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "submission status {} rejected by the schema: {error}",
                status.as_str()
            )
        });
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
