use std::time::Duration;

use hw_bot::app::{Actor, ErrorCode};
use hw_bot::db;
use hw_bot::domain::{InviteGeneration, StudentGroupId};
use sqlx::PgPool;

use crate::harness::{TestApp, fresh_database};

#[tokio::test]
async fn every_group_gets_an_invite_when_it_is_created() {
    let app = TestApp::start().await;

    let generation = db::group_invites::find_generation(&app.pool, app.fixture.group.id())
        .await
        .unwrap();

    assert_eq!(generation, Some(InviteGeneration::FIRST));
}

#[tokio::test]
async fn a_group_has_at_most_one_invite() {
    let app = TestApp::start().await;

    let duplicate = sqlx::query(
        "insert into student_group_invites (student_group_id, generation) values ($1, 2)",
    )
    .bind(app.fixture.group.id().as_uuid())
    .execute(&app.pool)
    .await;

    assert!(duplicate.is_err());
}

#[tokio::test]
async fn generation_stays_inside_the_u32_range() {
    let app = TestApp::start().await;

    for generation in [0i64, -1, i64::from(u32::MAX) + 1] {
        let stored = sqlx::query(
            "update student_group_invites set generation = $2 where student_group_id = $1",
        )
        .bind(app.fixture.group.id().as_uuid())
        .bind(generation)
        .execute(&app.pool)
        .await;

        assert!(stored.is_err(), "{generation} must be rejected");
    }
}

#[tokio::test]
async fn deleting_a_group_deletes_its_invite() {
    let pool = fresh_database().await;
    db::run_migrations(&pool).await.unwrap();
    let group_id = insert_bare_group(&pool, "DELETE-ME").await;

    sqlx::query("delete from student_groups where id = $1")
        .bind(group_id)
        .execute(&pool)
        .await
        .unwrap();

    let left: i64 = sqlx::query_scalar(
        "select count(*) from student_group_invites where student_group_id = $1",
    )
    .bind(group_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[tokio::test]
async fn the_migration_backfills_existing_groups() {
    static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

    let pool = fresh_database().await;
    db::run_migrations(&pool).await.unwrap();
    MIGRATOR.undo(&pool, 2).await.unwrap();
    sqlx::query(
        "insert into student_groups (code, education_level, study_year) values ('OLD', 'bachelor', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();

    db::run_migrations(&pool).await.unwrap();

    let generation: i64 = sqlx::query_scalar(
        "select generation from student_group_invites
         join student_groups on student_groups.id = student_group_invites.student_group_id
         where student_groups.code = 'OLD'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(generation, 1);
}

#[tokio::test]
async fn only_one_of_two_concurrent_rotations_wins() {
    let app = TestApp::start().await;
    let group_id = app.fixture.group.id();

    let (first, second) = tokio::join!(
        db::group_invites::rotate(&app.pool, group_id, InviteGeneration::FIRST),
        db::group_invites::rotate(&app.pool, group_id, InviteGeneration::FIRST),
    );

    assert_eq!(first.is_ok(), second.is_err(), "exactly one must win");
    assert_eq!(
        db::group_invites::find_generation(&app.pool, group_id)
            .await
            .unwrap(),
        Some(InviteGeneration::new(2).unwrap())
    );
}

/// Регистрация, успевшая взять `for share`, коммитится раньше ротации: инвайт нельзя обесценить
/// задним числом.
#[tokio::test]
async fn rotation_waits_for_a_registration_that_already_holds_the_lock() {
    let app = TestApp::start().await;
    let group_id = app.fixture.group.id();
    let mut registration = app.pool.begin().await.unwrap();

    let locked = db::group_invites::lock_group_for_registration(
        &mut registration,
        group_id,
        InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    assert!(locked.is_some());

    let pool = app.pool.clone();
    let rotation = tokio::spawn(async move {
        db::group_invites::rotate(&pool, group_id, InviteGeneration::FIRST).await
    });

    assert!(
        tokio::time::timeout(Duration::from_millis(300), rotation)
            .await
            .is_err(),
        "rotation must wait for the registration transaction"
    );
    registration.commit().await.unwrap();
}

#[tokio::test]
async fn a_rotated_invite_stops_resolving_the_group() {
    let app = TestApp::start().await;
    let group_id = app.fixture.group.id();
    db::group_invites::rotate(&app.pool, group_id, InviteGeneration::FIRST)
        .await
        .unwrap();

    let mut registration = app.pool.begin().await.unwrap();
    let locked = db::group_invites::lock_group_for_registration(
        &mut registration,
        group_id,
        InviteGeneration::FIRST,
    )
    .await
    .unwrap();

    assert!(locked.is_none());
}

#[tokio::test]
async fn a_link_issued_before_rotation_no_longer_registers() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    db::group_invites::rotate(&app.pool, app.fixture.group.id(), InviteGeneration::FIRST)
        .await
        .unwrap();

    app.register_student(app.next_telegram_id(), "Late", &token)
        .await
        .error(axum::http::StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn two_courses_of_one_group_share_a_single_link() {
    let app = TestApp::start().await;
    let second = app.add_subject_course("Databases").await;

    let first_invite = app
        .application
        .courses
        .get_group_invite(app.teacher_actor(), app.fixture.course.id())
        .await
        .unwrap();
    let second_invite = app
        .application
        .courses
        .get_group_invite(app.teacher_actor(), second.id())
        .await
        .unwrap();

    assert_eq!(first_invite.group.id(), second_invite.group.id());
    assert_eq!(first_invite.generation, second_invite.generation);
}

#[tokio::test]
async fn rotating_from_one_course_replaces_the_link_of_the_whole_group() {
    let app = TestApp::start().await;
    let second = app.add_subject_course("Databases").await;

    app.application
        .courses
        .rotate_group_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap();

    let invite = app
        .application
        .courses
        .get_group_invite(app.teacher_actor(), second.id())
        .await
        .unwrap();
    assert_eq!(invite.generation, InviteGeneration::new(2).unwrap());
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_read_the_link() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    let error = app
        .application
        .courses
        .get_group_invite(Actor::new(foreign.teacher.id()), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_rotate_the_link() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    let error = app
        .application
        .courses
        .rotate_group_invite(
            Actor::new(foreign.teacher.id()),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn an_admin_manages_the_link_of_any_group() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    let invite = app
        .application
        .courses
        .rotate_group_invite(
            Actor::new(admin.id()),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap();

    assert_eq!(invite.generation, InviteGeneration::new(2).unwrap());
}

#[tokio::test]
async fn a_stale_expected_generation_is_a_concurrent_modification() {
    let app = TestApp::start().await;
    db::group_invites::rotate(&app.pool, app.fixture.group.id(), InviteGeneration::FIRST)
        .await
        .unwrap();

    let error = app
        .application
        .courses
        .rotate_group_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::ConcurrentModification);
}

#[tokio::test]
async fn a_group_without_an_invite_row_never_falls_back_to_open_registration() {
    let app = TestApp::start().await;
    let token = app.group_invite_token().await;
    sqlx::query("delete from student_group_invites where student_group_id = $1")
        .bind(app.fixture.group.id().as_uuid())
        .execute(&app.pool)
        .await
        .unwrap();

    app.register_student(app.next_telegram_id(), "Ivan", &token)
        .await
        .error(axum::http::StatusCode::FORBIDDEN, "invite_invalid");
}

async fn insert_bare_group(pool: &PgPool, code: &str) -> uuid::Uuid {
    let group_id: uuid::Uuid = sqlx::query_scalar(
        "insert into student_groups (code, education_level, study_year)
         values ($1, 'bachelor', 1)
         returning id",
    )
    .bind(code)
    .fetch_one(pool)
    .await
    .unwrap();

    db::group_invites::ensure(pool, StudentGroupId::from_uuid(group_id))
        .await
        .unwrap();

    group_id
}
