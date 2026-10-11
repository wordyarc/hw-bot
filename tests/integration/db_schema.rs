use chrono::{Duration, Utc};
use hw_bot::db;
use hw_bot::domain::{
    AssignmentStatus, DeductedFrom, EducationLevel, FilePresentation, SubmissionKind,
    SubmissionStatus,
};
use sqlx::postgres::PgQueryResult;
use uuid::Uuid;

use crate::harness::{TestApp, fresh_database};

const INSERT_GROUP: &str = r#"
    insert into student_groups (code, education_level, study_year)
    values ($1, $2, $3)
"#;

#[tokio::test]
async fn migrations_apply_to_empty_postgres() -> anyhow::Result<()> {
    let pool = fresh_database().await;

    db::run_migrations(&pool).await?;
    db::validate_migrations(&pool).await?;

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
                teaching_course_id, subject_id, title, body, max_attempts, status, created_by,
                deadline_at, published_at
            )
            values ($1, $2, $3, 'body', 1, $4, $5, $6, $7)
            "#,
        )
        .bind(fixture.course.id().as_uuid())
        .bind(fixture.subject.id().as_uuid())
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
                assignment_id, student_id, kind, attempt_number, content, status,
                reviewed_by, reviewed_at, review_comment
            )
            values ($1, $2, 'regular', $3, 'https://example.test/pr', $4, $5, $6, $7)
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

    for kind in SubmissionKind::ALL {
        sqlx::query(
            "update submissions set kind = $3 where assignment_id = $1 and student_id = $2 and attempt_number = 1",
        )
        .bind(fixture.assignment.id().as_uuid())
        .bind(fixture.student.id().as_uuid())
        .bind(kind.as_str())
        .execute(&app.pool)
        .await
        .unwrap_or_else(|error| panic!("submission kind {} rejected: {error}", kind.as_str()));
    }

    let trainee = app.add_student("Deduction trainee").await;
    let graded = app
        .queued_submission(trainee.telegram_user_id().as_i64(), "pull/deducted_from")
        .await;
    app.review(graded, app.teacher(), "accept", "ok").await.ok();
    for deducted_from in DeductedFrom::ALL {
        let (work_score, course_total_penalty) = match deducted_from {
            DeductedFrom::WorkScore => (3, 0),
            DeductedFrom::CourseTotal => (4, 1),
        };
        store_grade(
            &app,
            graded,
            1,
            1,
            Some(deducted_from.as_str()),
            work_score,
            course_total_penalty,
        )
        .await
        .unwrap_or_else(|error| {
            panic!(
                "submission deducted_from {} rejected: {error}",
                deducted_from.as_str()
            )
        });
    }

    for deducted_from in DeductedFrom::ALL {
        sqlx::query(
            r#"
            update assignments
            set max_score = 5, late_period_days = 7, late_penalty_per_period = 1, late_max_periods = 4,
                late_deducted_from = $2
            where id = $1
            "#,
        )
        .bind(fixture.assignment.id().as_uuid())
        .bind(deducted_from.as_str())
        .execute(&app.pool)
        .await
        .unwrap_or_else(|error| panic!("assignment deducted_from {} rejected: {error}", deducted_from.as_str()));
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
                assignment_id, student_id, kind, attempt_number, content, status,
                reviewed_by, reviewed_at, review_comment
            )
            values ($1, $2, 'regular', 1, 'https://example.test/pr', $3, $4, now(), null)
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
                work_score = 4,
                course_total_penalty = 0
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

async fn accepted_submission(app: &TestApp) -> Uuid {
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "ok")
        .await
        .ok();

    submission
}

async fn store_grade(
    app: &TestApp,
    submission: Uuid,
    late_periods: i32,
    penalty: i16,
    deducted_from: Option<&str>,
    work_score: i16,
    course_total_penalty: i16,
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        r#"
        update submissions
        set raw_score = 4, grade_max_score = 5, late_periods = $2, penalty = $3,
            penalty_deducted_from = $4, work_score = $5, course_total_penalty = $6
        where id = $1
        "#,
    )
    .bind(submission)
    .bind(late_periods)
    .bind(penalty)
    .bind(deducted_from)
    .bind(work_score)
    .bind(course_total_penalty)
    .execute(&app.pool)
    .await
}

#[tokio::test]
async fn stored_grade_rejects_a_penalty_above_the_effective_limit() {
    let app = TestApp::start().await;
    let submission = accepted_submission(&app).await;

    let error = store_grade(&app, submission, 2, 101, Some("course_total"), 4, 101)
        .await
        .unwrap_err();

    assert_check(
        error,
        Some("submissions_grade_belongs_to_accepted_submission"),
    );
}

#[tokio::test]
async fn a_late_policy_is_stored_only_together_with_its_deduction() {
    let app = TestApp::start().await;
    let update = |period: Option<i16>, deducted_from: Option<&'static str>| {
        sqlx::query(
            r#"
            update assignments
            set max_score = 5, late_period_days = $2, late_penalty_per_period = $2, late_max_periods = $2,
                late_deducted_from = $3
            where id = $1
            "#,
        )
        .bind(app.assignment())
        .bind(period)
        .bind(deducted_from)
        .execute(&app.pool)
    };

    for (period, deducted_from) in [(Some(1), None), (None, Some("course_total"))] {
        assert_check(
            update(period, deducted_from).await.unwrap_err(),
            Some("assignments_late_policy_is_complete"),
        );
    }
    update(Some(1), Some("course_total"))
        .await
        .expect("a complete policy is valid");
}

#[tokio::test]
async fn only_a_late_grade_says_where_its_penalty_is_deducted_from() {
    let app = TestApp::start().await;
    let submission = accepted_submission(&app).await;

    for (late_periods, penalty, deducted_from) in [(0, 0, Some("work_score")), (2, 2, None)] {
        let error = store_grade(&app, submission, late_periods, penalty, deducted_from, 4, 0)
            .await
            .unwrap_err();
        assert_check(
            error,
            Some("submissions_penalty_deducted_from_marks_a_late_grade"),
        );
    }
    store_grade(&app, submission, 2, 2, Some("work_score"), 2, 0)
        .await
        .expect("a late grade with its deducted_from is valid");
}

#[tokio::test]
async fn a_stored_work_score_follows_where_the_penalty_is_deducted_from() {
    let app = TestApp::start().await;
    let submission = accepted_submission(&app).await;

    for (deducted_from, work_score, course_total_penalty) in
        [("work_score", 4, 0), ("course_total", 2, 2)]
    {
        let error = store_grade(
            &app,
            submission,
            1,
            2,
            Some(deducted_from),
            work_score,
            course_total_penalty,
        )
        .await
        .unwrap_err();
        assert_check(
            error,
            Some("submissions_grade_follows_penalty_deducted_from"),
        );
    }
}

#[tokio::test]
async fn a_stored_course_total_penalty_follows_where_the_penalty_is_deducted_from() {
    let app = TestApp::start().await;
    let submission = accepted_submission(&app).await;

    for (deducted_from, work_score, course_total_penalty) in
        [("work_score", 2, 2), ("course_total", 4, 0)]
    {
        let error = store_grade(
            &app,
            submission,
            1,
            2,
            Some(deducted_from),
            work_score,
            course_total_penalty,
        )
        .await
        .unwrap_err();
        assert_check(
            error,
            Some("submissions_grade_follows_penalty_deducted_from"),
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

    assert_check(
        valid(None, None, 1).await.unwrap_err(),
        Some("stored_files_check"),
    );
    assert_check(
        valid(
            Some(app.fixture.assignment.id().as_uuid()),
            Some(app.queued_submission(app.student(), "pull/1").await),
            2,
        )
        .await
        .unwrap_err(),
        Some("stored_files_check"),
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
async fn file_positions_are_unique_within_each_owner() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    for (assignment, submission, constraint) in [
        (
            Some(app.assignment()),
            None,
            "stored_files_assignment_position_idx",
        ),
        (
            None,
            Some(submission),
            "stored_files_submission_position_idx",
        ),
    ] {
        let insert = || {
            sqlx::query(
                "insert into stored_files (
                    assignment_id, submission_id, position, presentation, filename,
                    size_bytes, sha256, content
                 ) values ($1, $2, 1, 'document', 'file.txt', 0, $3, $4)",
            )
            .bind(assignment)
            .bind(submission)
            .bind(vec![0_u8; 32])
            .bind(Vec::<u8>::new())
            .execute(&app.pool)
        };
        insert().await.unwrap();
        let error = insert().await.unwrap_err();
        let database = error.as_database_error().unwrap();
        assert_eq!(database.code().as_deref(), Some("23505"), "{error}");
        assert_eq!(database.constraint(), Some(constraint), "{error}");
    }
}

#[tokio::test]
async fn stored_file_size_and_checksum_shape_are_hard_invariants() -> anyhow::Result<()> {
    let app = TestApp::start().await;

    for (size, sha256, constraint) in [
        (2_i64, vec![0_u8; 32], "stored_files_check1"),
        (1_i64, vec![0_u8; 31], "stored_files_sha256_check"),
    ] {
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

        assert_check(result.unwrap_err(), Some(constraint));
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
                teaching_course_id, subject_id, title, body, max_attempts, status, created_by,
                deadline_at, published_at
            )
            values ($1, $2, $3, 'body', 1, $4, $5, $6, $7)
            "#,
        )
        .bind(fixture.course.id().as_uuid())
        .bind(fixture.subject.id().as_uuid())
        .bind(format!("invalid-{status}"))
        .bind(status)
        .bind(fixture.teacher.id().as_uuid())
        .bind(Utc::now() + Duration::days(7))
        .bind(published_at)
        .execute(&app.pool)
        .await;

        assert_check(result.unwrap_err(), Some("assignments_check1"));
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
            teaching_course_id, subject_id, title, body, max_attempts, status, created_by,
            deadline_at
        )
        values ($1, $2, 'title', $3, 1, 'draft', $4, $5)
        "#,
    )
    .bind(fixture.course.id().as_uuid())
    .bind(fixture.subject.id().as_uuid())
    .bind("я".repeat(4097))
    .bind(fixture.teacher.id().as_uuid())
    .bind(Utc::now() + Duration::days(7))
    .execute(&app.pool)
    .await;

    assert_check(result.unwrap_err(), Some("assignments_body_check"));
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
    assert_check(group.unwrap_err(), None);

    let assignment = sqlx::query(
        r#"
        insert into assignments (
            teaching_course_id, subject_id, title, body, max_attempts, status, created_by,
            deadline_at
        )
        values ($1, $2, 'bogus', 'body', 1, 'postponed', $3, $4)
        "#,
    )
    .bind(fixture.course.id().as_uuid())
    .bind(fixture.subject.id().as_uuid())
    .bind(fixture.teacher.id().as_uuid())
    .bind(Utc::now() + Duration::days(7))
    .execute(&app.pool)
    .await;
    assert_check(assignment.unwrap_err(), None);

    for (kind, status) in [("regular", "maybe"), ("bonus", "pending_review")] {
        let submission = sqlx::query(
            r#"
            insert into submissions (assignment_id, student_id, kind, attempt_number, content, status)
            values ($1, $2, $3, 1, 'https://example.test/pr', $4)
            "#,
        )
        .bind(fixture.assignment.id().as_uuid())
        .bind(fixture.student.id().as_uuid())
        .bind(kind)
        .bind(status)
        .execute(&app.pool)
        .await;
        assert_check(submission.unwrap_err(), None);
    }

    let deducted_from = sqlx::query(
        r#"
        update assignments
        set max_score = 5, late_period_days = 7, late_penalty_per_period = 1, late_max_periods = 4,
            late_deducted_from = 'everything'
        where id = $1
        "#,
    )
    .bind(fixture.assignment.id().as_uuid())
    .execute(&app.pool)
    .await;
    assert_check(deducted_from.unwrap_err(), None);

    Ok(())
}

async fn store_makeup_terms(
    app: &TestApp,
    max_score: Option<i16>,
    offered: Option<bool>,
    credit: Option<i16>,
) -> Result<PgQueryResult, sqlx::Error> {
    sqlx::query(
        "update assignments set max_score = $2, makeup_offered = $3, makeup_credit = $4 where id = $1",
    )
    .bind(app.assignment())
    .bind(max_score)
    .bind(offered)
    .bind(credit)
    .execute(&app.pool)
    .await
}

#[tokio::test]
async fn a_stored_makeup_credit_lies_within_the_scale_of_an_offered_makeup() {
    let app = TestApp::start().await;

    for (max_score, offered, credit) in [
        (Some(10), Some(true), Some(11)),
        (Some(10), None, Some(6)),
        (Some(10), Some(false), Some(6)),
        (None, Some(true), Some(6)),
    ] {
        let error = store_makeup_terms(&app, max_score, offered, credit)
            .await
            .unwrap_err();
        assert_check(error, Some("assignments_makeup_credit_fits_the_scale"));
    }
}

#[tokio::test]
async fn an_offered_makeup_carries_a_credit_exactly_on_a_scored_scale() {
    let app = TestApp::start().await;

    let error = store_makeup_terms(&app, Some(10), Some(true), None)
        .await
        .unwrap_err();
    assert_check(error, Some("assignments_makeup_credit_follows_the_scale"));
    for (max_score, offered, credit) in [
        (Some(10), Some(true), Some(6)),
        (None, Some(true), None),
        (Some(10), Some(false), None),
        (None, None, None),
    ] {
        store_makeup_terms(&app, max_score, offered, credit)
            .await
            .unwrap_or_else(|error| panic!("{max_score:?} {offered:?} {credit:?}: {error}"));
    }
}

#[tokio::test]
async fn a_makeup_grade_has_no_lateness() {
    let app = TestApp::start().await;
    let submission = accepted_submission(&app).await;
    sqlx::query("update submissions set kind = 'makeup' where id = $1")
        .bind(submission)
        .execute(&app.pool)
        .await
        .expect("an ungraded makeup is valid");

    let error = store_grade(&app, submission, 2, 2, Some("work_score"), 2, 0)
        .await
        .unwrap_err();
    assert_check(error, Some("submissions_makeup_is_not_late"));
    store_grade(&app, submission, 0, 0, None, 4, 0)
        .await
        .expect("a makeup grade without lateness is valid");
}

#[tokio::test]
async fn course_makeup_settings_are_stored_whole_and_end_after_their_opening() {
    let app = TestApp::start().await;
    let update = |until: Option<chrono::DateTime<Utc>>, attempts: Option<i16>, opened_at| {
        sqlx::query(
            "update teaching_courses set makeup_until = $2, makeup_attempts = $3, makeup_opened_at = $4 where id = $1",
        )
        .bind(app.course())
        .bind(until)
        .bind(attempts)
        .bind(opened_at)
        .execute(&app.pool)
    };
    let now = Utc::now();

    for (until, attempts, opened_at) in [
        (Some(now + Duration::days(30)), None, Some(now)),
        (None, Some(3), Some(now)),
        (Some(now + Duration::days(30)), Some(3), None),
    ] {
        assert_check(
            update(until, attempts, opened_at).await.unwrap_err(),
            Some("teaching_courses_makeup_is_complete"),
        );
    }
    assert_check(
        update(Some(now), Some(3), Some(now)).await.unwrap_err(),
        Some("teaching_courses_makeup_ends_after_opening"),
    );
    update(Some(now + Duration::days(30)), Some(3), Some(now))
        .await
        .expect("complete settings are valid");
}

#[tokio::test]
async fn only_one_accepted_submission_of_each_kind_exists_per_assignment() {
    let app = TestApp::start().await;
    accepted_submission(&app).await;
    let insert = |kind: &'static str, attempt: i16| {
        sqlx::query(
            r#"
            insert into submissions (
                assignment_id, student_id, kind, attempt_number, content, status,
                reviewed_by, reviewed_at, review_comment
            )
            values ($1, $2, $3, $4, 'https://example.test/pr', 'accepted', $5, now(), 'ok')
            "#,
        )
        .bind(app.assignment())
        .bind(app.fixture.student.id().as_uuid())
        .bind(kind)
        .bind(attempt)
        .bind(app.fixture.teacher.id().as_uuid())
        .execute(&app.pool)
    };

    let error = insert("regular", 2).await.unwrap_err();
    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23505"), "{error}");
    assert_eq!(
        database.constraint(),
        Some("submissions_one_accepted_per_student_assignment_kind_idx")
    );
    insert("makeup", 2)
        .await
        .expect("an accepted makeup sits next to an accepted regular submission");
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

        assert_check(over_limit.unwrap_err(), Some("student_groups_check"));
    }

    Ok(())
}

#[track_caller]
fn assert_check(error: sqlx::Error, constraint: Option<&str>) {
    let database = error.as_database_error().expect("a PostgreSQL error");
    assert_eq!(database.code().as_deref(), Some("23514"), "{error}");
    if let Some(constraint) = constraint {
        assert_eq!(database.constraint(), Some(constraint), "{error}");
    }
}

#[tokio::test]
async fn a_group_belongs_to_at_most_one_course_of_a_subject() {
    let app = TestApp::start().await;
    let second = app.add_foreign_course_on(&app.fixture.subject).await.course;

    let error =
        db::academic::add_teaching_course_group(&app.pool, second.id(), app.fixture.group.id())
            .await
            .unwrap_err();

    let db::DbError::Database(error) = error else {
        panic!("a constraint violation, got {error:?}");
    };
    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23505"), "{error}");
    assert_eq!(
        database.constraint(),
        Some("teaching_course_groups_subject_id_student_group_id_key")
    );
}

#[tokio::test]
async fn an_assignment_is_issued_only_to_groups_of_its_course() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    let error = sqlx::query(
        "insert into assignment_groups (assignment_id, teaching_course_id, student_group_id)
         values ($1, $2, $3)",
    )
    .bind(app.fixture.assignment.id().as_uuid())
    .bind(app.fixture.course.id().as_uuid())
    .bind(foreign.group.id().as_uuid())
    .execute(&app.pool)
    .await
    .unwrap_err();

    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23503"), "{error}");
    assert_eq!(
        database.constraint(),
        Some("assignment_groups_teaching_course_id_student_group_id_fkey")
    );
}

#[tokio::test]
async fn deleting_a_course_deletes_its_invite_and_groups() {
    let app = TestApp::start().await;
    let course = app.add_foreign_course().await.course;

    sqlx::query("delete from teaching_courses where id = $1")
        .bind(course.id().as_uuid())
        .execute(&app.pool)
        .await
        .unwrap();

    let (invites, groups): (i64, i64) = sqlx::query_as(
        "select
            (select count(*) from teaching_course_invites where teaching_course_id = $1),
            (select count(*) from teaching_course_groups where teaching_course_id = $1)",
    )
    .bind(course.id().as_uuid())
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!((invites, groups), (0, 0));
}

#[tokio::test]
async fn the_invite_migration_backfills_courses_created_without_an_invite() {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    let pool = fresh_database().await;
    db::run_migrations(&pool).await.unwrap();
    MIGRATOR.undo(&pool, 2).await.unwrap();
    let course_id: Uuid = sqlx::query_scalar(
        "with subject as (insert into subjects (name) values ('Old') returning id)
         insert into teaching_courses (subject_id) select id from subject returning id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    db::run_migrations(&pool).await.unwrap();

    let generation: i64 = sqlx::query_scalar(
        "select generation from teaching_course_invites where teaching_course_id = $1",
    )
    .bind(course_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(generation, 1);
}
