use std::time::Duration;

use chrono::Utc;
use hw_bot::app::{Actor, ErrorCode, RegisterStudent};
use hw_bot::db;
use hw_bot::domain::{InviteGeneration, StudentGroupId, TeachingCourseId, TelegramUserId};
use hw_bot::invites::VerifiedInvite;

use crate::harness::{TestApp, wait_for_blocked_connection};

#[tokio::test]
async fn every_course_gets_an_invite_when_it_is_created() {
    let app = TestApp::start().await;

    let generation = db::course_invites::find_generation(&app.pool, app.fixture.course.id())
        .await
        .unwrap();

    assert_eq!(generation, Some(InviteGeneration::FIRST));
}

#[tokio::test]
async fn a_course_has_at_most_one_invite() {
    let app = TestApp::start().await;

    let duplicate = sqlx::query(
        "insert into teaching_course_invites (teaching_course_id, generation) values ($1, 2)",
    )
    .bind(app.fixture.course.id().as_uuid())
    .execute(&app.pool)
    .await;

    let error = duplicate.unwrap_err();
    let database = error.as_database_error().unwrap();
    assert_eq!(database.code().as_deref(), Some("23505"), "{error}");
    assert_eq!(database.constraint(), Some("teaching_course_invites_pkey"));
}

#[tokio::test]
async fn generation_stays_inside_the_u32_range() {
    let app = TestApp::start().await;

    for generation in [0i64, -1, i64::from(u32::MAX) + 1] {
        let stored = sqlx::query(
            "update teaching_course_invites set generation = $2 where teaching_course_id = $1",
        )
        .bind(app.fixture.course.id().as_uuid())
        .bind(generation)
        .execute(&app.pool)
        .await;

        let error = stored.unwrap_err();
        let database = error.as_database_error().unwrap();
        assert_eq!(
            database.code().as_deref(),
            Some("23514"),
            "{generation}: {error}"
        );
        assert_eq!(
            database.constraint(),
            Some("teaching_course_invites_generation_check")
        );
    }
}

/// Ротация не отзывает инвайт у регистрации, которая уже получила `for share`:
/// сначала должна завершиться её транзакция.
#[tokio::test]
async fn rotation_waits_for_a_registration_that_already_holds_the_lock() {
    let app = TestApp::start().await;
    let course_id = app.fixture.course.id();
    let mut registration = app.pool.begin().await.unwrap();

    let blocker = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *registration)
        .await
        .unwrap();

    let locked = db::course_invites::lock_course_for_registration(
        &mut registration,
        course_id,
        InviteGeneration::FIRST,
    )
    .await
    .unwrap();
    assert!(locked.is_some());

    let pool = app.pool.clone();
    let rotation = tokio::spawn(async move {
        db::course_invites::rotate(&pool, course_id, InviteGeneration::FIRST).await
    });

    wait_for_blocked_connection(&app.pool, blocker).await;
    assert!(!rotation.is_finished());
    registration.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), rotation)
        .await
        .expect("rotation continues after registration commits")
        .expect("rotation task does not panic")
        .expect("rotation succeeds");
    assert_eq!(
        db::course_invites::find_generation(&app.pool, course_id)
            .await
            .unwrap(),
        Some(InviteGeneration::new(2).unwrap())
    );
}

#[tokio::test]
async fn a_rotated_invite_stops_resolving_the_course() {
    let app = TestApp::start().await;
    let course_id = app.fixture.course.id();
    db::course_invites::rotate(&app.pool, course_id, InviteGeneration::FIRST)
        .await
        .unwrap();

    let mut registration = app.pool.begin().await.unwrap();
    let locked = db::course_invites::lock_course_for_registration(
        &mut registration,
        course_id,
        InviteGeneration::FIRST,
    )
    .await
    .unwrap();

    assert!(locked.is_none());
}

#[tokio::test]
async fn a_link_issued_before_rotation_no_longer_registers() {
    let app = TestApp::start().await;
    db::course_invites::rotate(&app.pool, app.fixture.course.id(), InviteGeneration::FIRST)
        .await
        .unwrap();

    assert_eq!(register(&app, None).await, Err(ErrorCode::InviteInvalid));
}

#[tokio::test]
async fn a_signed_invite_for_an_unknown_course_does_not_register() {
    let app = TestApp::start().await;
    let invite = app
        .invites
        .decode(
            &app.invites
                .encode(TeachingCourseId::new(), InviteGeneration::FIRST),
        )
        .unwrap();

    assert_eq!(
        register_with(&app, invite, None).await,
        Err(ErrorCode::InviteInvalid)
    );
}

#[tokio::test]
async fn a_course_has_one_link_for_all_of_its_groups() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;

    let invite = app
        .application
        .courses
        .get_course_invite(app.teacher_actor(), app.fixture.course.id())
        .await
        .unwrap();

    let mut codes: Vec<&str> = invite
        .groups
        .iter()
        .map(|group| group.code().as_str())
        .collect();
    codes.sort_unstable();
    let mut expected = vec![app.fixture.group.code().as_str(), second.code().as_str()];
    expected.sort_unstable();
    assert_eq!(codes, expected);
    assert_eq!(invite.subject.id(), app.fixture.subject.id());
}

#[tokio::test]
async fn rotating_the_link_of_one_course_keeps_the_link_of_another_course_of_the_group() {
    let app = TestApp::start().await;
    let second = app.add_subject_course("Databases").await;
    let kept = app.invite_token(second.id()).await;

    app.application
        .courses
        .rotate_course_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap();

    let invite = app
        .application
        .courses
        .get_course_invite(app.teacher_actor(), second.id())
        .await
        .unwrap();
    assert_eq!(invite.generation, InviteGeneration::FIRST);
    app.register_student(app.next_telegram_id(), "Ivan", &kept)
        .await
        .created();
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_read_the_link() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    let error = app
        .application
        .courses
        .get_course_invite(Actor::new(foreign.teacher.id()), app.fixture.course.id())
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
        .rotate_course_invite(
            Actor::new(foreign.teacher.id()),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn an_admin_manages_the_link_of_any_course() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    let invite = app
        .application
        .courses
        .rotate_course_invite(
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
    db::course_invites::rotate(&app.pool, app.fixture.course.id(), InviteGeneration::FIRST)
        .await
        .unwrap();

    let error = app
        .application
        .courses
        .rotate_course_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::ConcurrentModification);
}

#[tokio::test]
async fn a_course_without_an_invite_row_never_falls_back_to_open_registration() {
    let app = TestApp::start().await;
    sqlx::query("delete from teaching_course_invites where teaching_course_id = $1")
        .bind(app.fixture.course.id().as_uuid())
        .execute(&app.pool)
        .await
        .unwrap();

    assert_eq!(register(&app, None).await, Err(ErrorCode::InviteInvalid));
}

fn invite(app: &TestApp) -> VerifiedInvite {
    let generation = InviteGeneration::FIRST;
    app.invites
        .decode(&app.invites.encode(app.fixture.course.id(), generation))
        .unwrap()
}

async fn register(
    app: &TestApp,
    student_group_id: Option<StudentGroupId>,
) -> Result<StudentGroupId, ErrorCode> {
    register_with(app, invite(app), student_group_id).await
}

async fn register_with(
    app: &TestApp,
    invite: VerifiedInvite,
    student_group_id: Option<StudentGroupId>,
) -> Result<StudentGroupId, ErrorCode> {
    app.application
        .users
        .register_student(RegisterStudent {
            telegram_user_id: TelegramUserId::new(app.next_telegram_id()).unwrap(),
            display_name: "Ivan".to_owned(),
            invite,
            student_group_id,
            registered_at: Utc::now(),
        })
        .await
        .map(|registered| registered.student_group.unwrap().id())
        .map_err(|error| error.code())
}

#[tokio::test]
async fn a_student_registers_into_the_chosen_group_of_the_course() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;

    assert_eq!(register(&app, Some(second.id())).await, Ok(second.id()));
}

#[tokio::test]
async fn a_group_outside_the_course_is_invalid_input() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    assert_eq!(
        register(&app, Some(foreign.group.id())).await,
        Err(ErrorCode::InvalidInput)
    );
}

#[tokio::test]
async fn a_course_of_one_group_registers_without_a_chosen_group() {
    let app = TestApp::start().await;

    assert_eq!(register(&app, None).await, Ok(app.fixture.group.id()));
}

#[tokio::test]
async fn a_course_of_several_groups_requires_a_chosen_group() {
    let app = TestApp::start().await;
    app.add_group_to_course(&app.fixture.course).await;

    assert_eq!(register(&app, None).await, Err(ErrorCode::InvalidInput));
}
