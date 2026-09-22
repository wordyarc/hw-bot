use chrono::{Duration, Utc};
use hw_bot::db;
use hw_bot::domain::{AssignmentStatus, EducationLevel, FilePresentation, SubmissionStatus};
use sqlx::PgPool;

use crate::harness::{TestApp, fresh_database};

const INSERT_GROUP: &str = r#"
    insert into student_groups (code, education_level, study_year)
    values ($1, $2, $3)
"#;

#[tokio::test]
async fn migrations_apply_to_empty_postgres() -> anyhow::Result<()> {
    let pool = fresh_database().await;

    db::run_migrations(&pool).await?;

    for table_name in [
        "student_groups",
        "subjects",
        "users",
        "teaching_courses",
        "teaching_course_teachers",
        "assignments",
        "submissions",
        "stored_files",
        "student_assignment_progress",
        "review_notification_outbox",
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
        "review_notification_outbox_pending_idx",
        "stored_files_assignment_position_idx",
        "stored_files_submission_position_idx",
    ] {
        assert!(
            index_exists(&pool, index_name).await?,
            "missing index {index_name}"
        );
    }

    Ok(())
}

#[tokio::test]
async fn migrated_database_matches_the_embedded_migration_set() -> anyhow::Result<()> {
    let app = TestApp::start().await;

    db::validate_migrations(&app.pool).await?;

    Ok(())
}

#[tokio::test]
async fn database_without_migrations_is_not_ready() {
    let pool = fresh_database().await;

    assert!(db::validate_migrations(&pool).await.is_err());
}

/// CHECK-ограничения миграции должны принимать те же имена, что заданы доменным `str_enum!`.
#[tokio::test]
async fn check_constraints_accept_every_domain_enum_variant() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let fixture = &app.fixture;

    for level in EducationLevel::ALL {
        sqlx::query(INSERT_GROUP)
            .bind(format!("ok-{}", level.as_str()))
            .bind(level.as_str())
            .bind(i16::from(level.max_study_year()))
            .execute(&app.pool)
            .await
            .unwrap_or_else(|error| panic!("education level {} rejected: {error}", level.as_str()));
    }

    for status in AssignmentStatus::ALL {
        sqlx::query(
            r#"
            insert into assignments (
                teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
                status, created_by, deadline_at, published_at
            )
            values ($1, $2, $3, $4, 'body', 1, $5, $6, $7, $8)
            "#,
        )
        .bind(fixture.course.id().as_uuid())
        .bind(fixture.subject.id().as_uuid())
        .bind(fixture.group.id().as_uuid())
        .bind(format!("assignment-{}", status.as_str()))
        .bind(status.as_str())
        .bind(fixture.teacher.id().as_uuid())
        .bind(Utc::now() + Duration::days(7))
        .bind((*status == AssignmentStatus::Published).then(Utc::now))
        .execute(&app.pool)
        .await
        .unwrap_or_else(|error| panic!("assignment status {} rejected: {error}", status.as_str()));
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
        .execute(&app.pool)
        .await
        .unwrap_or_else(|error| panic!("submission status {} rejected: {error}", status.as_str()));
    }

    for (position, presentation) in FilePresentation::ALL.iter().enumerate() {
        sqlx::query(
            r#"
            insert into stored_files (
                assignment_id, position, presentation, filename, size_bytes, sha256, content
            )
            values ($1, $2, $3, $4, 0, $5, $6)
            "#,
        )
        .bind(fixture.assignment.id().as_uuid())
        .bind(position as i64 + 1)
        .bind(presentation.as_str())
        .bind(format!("{}.bin", presentation.as_str()))
        .bind([0_u8; 32].as_slice())
        .bind(Vec::<u8>::new())
        .execute(&app.pool)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "file presentation {} rejected: {error}",
                presentation.as_str()
            )
        });
    }

    Ok(())
}

#[tokio::test]
async fn completed_review_requires_a_comment() {
    let app = TestApp::start().await;

    for status in [SubmissionStatus::Accepted, SubmissionStatus::Rejected] {
        let error = sqlx::query(
            r#"
            insert into submissions (
                assignment_id, student_id, attempt_number, content, status,
                reviewed_by, reviewed_at, review_comment
            )
            values ($1, $2, 1, 'https://example.test/pr', $3, $4, now(), null)
            "#,
        )
        .bind(app.fixture.assignment.id().as_uuid())
        .bind(app.fixture.student.id().as_uuid())
        .bind(status.as_str())
        .bind(app.fixture.teacher.id().as_uuid())
        .execute(&app.pool)
        .await
        .expect_err("completed review without a comment must be rejected");

        assert_eq!(
            error.as_database_error().and_then(|error| error.code()),
            Some("23514".into()),
            "status: {}",
            status.as_str()
        );
    }
}

#[tokio::test]
async fn stored_grade_requires_the_frozen_scale_and_raw_score_within_it() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    let update = |max_score: Option<i16>| {
        sqlx::query(
            r#"
            update submissions
            set status = 'accepted',
                reviewed_by = $2,
                reviewed_at = now(),
                review_comment = 'ok',
                raw_score = 4,
                grade_max_score = $3,
                late_periods = 0,
                penalty = 0,
                final_score = 4
            where id = $1
            "#,
        )
        .bind(submission)
        .bind(app.fixture.teacher.id().as_uuid())
        .bind(max_score)
        .execute(&app.pool)
    };

    for max_score in [None, Some(3)] {
        let error = update(max_score)
            .await
            .expect_err("an incomplete or too small frozen scale must be rejected");
        assert_eq!(
            error.as_database_error().and_then(|error| error.code()),
            Some("23514".into())
        );
    }

    update(Some(5))
        .await
        .expect("a complete grade within its frozen scale is valid");
}

#[tokio::test]
async fn stored_grade_rejects_inconsistent_arithmetic_and_excessive_penalty() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "ok")
        .await
        .ok();
    for (penalty, final_score) in [(2_i16, 4_i16), (101, 0)] {
        let error = sqlx::query("update submissions set raw_score = 4, grade_max_score = 5, late_periods = 2, penalty = $2, final_score = $3 where id = $1")
            .bind(submission).bind(penalty).bind(final_score).execute(&app.pool).await.unwrap_err();
        assert_eq!(
            error
                .as_database_error()
                .and_then(|error| error.constraint()),
            Some("submissions_grade_belongs_to_accepted_submission")
        );
    }
}

#[tokio::test]
async fn stored_file_requires_exactly_one_owner() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let valid = |assignment: Option<uuid::Uuid>, submission: Option<uuid::Uuid>, position: i64| {
        sqlx::query(
            r#"
            insert into stored_files (
                assignment_id, submission_id, position, presentation, filename,
                size_bytes, sha256, content
            )
            values ($1, $2, $3, 'document', 'file.bin', 0, $4, $5)
            "#,
        )
        .bind(assignment)
        .bind(submission)
        .bind(position)
        .bind([0_u8; 32].as_slice())
        .bind(Vec::<u8>::new())
        .execute(&app.pool)
    };

    assert!(valid(None, None, 1).await.is_err());
    assert!(
        valid(
            Some(app.fixture.assignment.id().as_uuid()),
            Some(app.queued_submission(app.student(), "pull/1").await),
            2,
        )
        .await
        .is_err()
    );
    assert!(
        valid(Some(app.fixture.assignment.id().as_uuid()), None, 3)
            .await
            .is_ok()
    );

    Ok(())
}

#[tokio::test]
async fn stored_file_schema_has_no_installation_size_or_count_limit() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let content = vec![0x5a_u8; 5 * 1024 * 1024 + 1];

    for position in 1_i64..=6 {
        sqlx::query(
            r#"
            insert into stored_files (
                assignment_id, position, presentation, filename,
                size_bytes, sha256, content
            )
            values ($1, $2, 'document', $3, $4, $5, $6)
            "#,
        )
        .bind(app.fixture.assignment.id().as_uuid())
        .bind(position)
        .bind(format!("file-{position}.bin"))
        .bind(content.len() as i64)
        .bind([0_u8; 32].as_slice())
        .bind(&content)
        .execute(&app.pool)
        .await?;
    }

    let count: i64 =
        sqlx::query_scalar("select count(*) from stored_files where assignment_id = $1")
            .bind(app.fixture.assignment.id().as_uuid())
            .fetch_one(&app.pool)
            .await?;
    assert_eq!(count, 6);
    Ok(())
}

#[tokio::test]
async fn stored_file_size_and_checksum_shape_are_hard_invariants() -> anyhow::Result<()> {
    let app = TestApp::start().await;

    for (size, sha256) in [(2_i64, vec![0_u8; 32]), (1_i64, vec![0_u8; 31])] {
        let result = sqlx::query(
            r#"
            insert into stored_files (
                assignment_id, position, presentation, filename,
                size_bytes, sha256, content
            )
            values ($1, $2, 'document', 'file.bin', $3, $4, $5)
            "#,
        )
        .bind(app.fixture.assignment.id().as_uuid())
        .bind(size)
        .bind(size)
        .bind(sha256)
        .bind(vec![1_u8])
        .execute(&app.pool)
        .await;

        assert!(result.is_err());
    }
    Ok(())
}

#[tokio::test]
async fn assignment_status_requires_the_matching_published_at_shape() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let fixture = &app.fixture;

    for (status, published_at) in [("draft", Some(Utc::now())), ("published", None)] {
        let result = sqlx::query(
            r#"
            insert into assignments (
                teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
                status, created_by, deadline_at, published_at
            )
            values ($1, $2, $3, $4, 'body', 1, $5, $6, $7, $8)
            "#,
        )
        .bind(fixture.course.id().as_uuid())
        .bind(fixture.subject.id().as_uuid())
        .bind(fixture.group.id().as_uuid())
        .bind(format!("invalid-{status}"))
        .bind(status)
        .bind(fixture.teacher.id().as_uuid())
        .bind(Utc::now() + Duration::days(7))
        .bind(published_at)
        .execute(&app.pool)
        .await;

        assert!(result.is_err(), "schema accepted invalid {status} shape");
    }

    Ok(())
}

#[tokio::test]
async fn schema_rejects_user_text_over_the_product_limit() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let fixture = &app.fixture;

    let result = sqlx::query(
        r#"
        insert into assignments (
            teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, deadline_at
        )
        values ($1, $2, $3, 'title', $4, 1, 'draft', $5, $6)
        "#,
    )
    .bind(fixture.course.id().as_uuid())
    .bind(fixture.subject.id().as_uuid())
    .bind(fixture.group.id().as_uuid())
    .bind("я".repeat(4097))
    .bind(fixture.teacher.id().as_uuid())
    .bind(Utc::now() + Duration::days(7))
    .execute(&app.pool)
    .await;

    assert!(
        result.is_err(),
        "schema accepted an oversized assignment body"
    );
    Ok(())
}

#[tokio::test]
async fn check_constraints_reject_unknown_enum_values() -> anyhow::Result<()> {
    let app = TestApp::start().await;
    let fixture = &app.fixture;

    let group = sqlx::query(INSERT_GROUP)
        .bind("bogus-level")
        .bind("wizardry")
        .bind(1_i16)
        .execute(&app.pool)
        .await;
    assert!(
        group.is_err(),
        "the schema accepts an unknown education level"
    );

    let assignment = sqlx::query(
        r#"
        insert into assignments (
            teaching_course_id, subject_id, student_group_id, title, body, max_attempts,
            status, created_by, deadline_at
        )
        values ($1, $2, $3, 'bogus', 'body', 1, 'postponed', $4, $5)
        "#,
    )
    .bind(fixture.course.id().as_uuid())
    .bind(fixture.subject.id().as_uuid())
    .bind(fixture.group.id().as_uuid())
    .bind(fixture.teacher.id().as_uuid())
    .bind(Utc::now() + Duration::days(7))
    .execute(&app.pool)
    .await;
    assert!(
        assignment.is_err(),
        "the schema accepts an unknown assignment status"
    );

    let submission = sqlx::query(
        r#"
        insert into submissions (assignment_id, student_id, attempt_number, content, status)
        values ($1, $2, 1, 'https://example.test/pr', 'maybe')
        "#,
    )
    .bind(fixture.assignment.id().as_uuid())
    .bind(fixture.student.id().as_uuid())
    .execute(&app.pool)
    .await;
    assert!(
        submission.is_err(),
        "the schema accepts an unknown submission status"
    );

    Ok(())
}

/// Верхняя граница курса задана и в `max_study_year()`, и в CHECK миграции; значения должны совпадать.
#[tokio::test]
async fn study_year_limits_match_the_domain() -> anyhow::Result<()> {
    let app = TestApp::start().await;

    for level in EducationLevel::ALL {
        let max = i16::from(level.max_study_year());

        let over_limit = sqlx::query(INSERT_GROUP)
            .bind(format!("over-{}", level.as_str()))
            .bind(level.as_str())
            .bind(max + 1)
            .execute(&app.pool)
            .await;

        assert!(
            over_limit.is_err(),
            "the schema accepts {} year {}, but max_study_year() says {max}",
            level.as_str(),
            max + 1
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
