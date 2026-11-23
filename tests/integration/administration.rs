use std::collections::BTreeSet;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use hw_bot::app::{Actor, ConfigureMakeup, ErrorCode, MakeupSettings, SubmitHomework};
use hw_bot::db;
use hw_bot::domain::{StudentGroup, TeachingCourse, User};
use sqlx::{Postgres, Transaction};

use crate::harness::{TestApp, wait_for_blocked_connection};

async fn blocking_transaction(app: &TestApp) -> (Transaction<'static, Postgres>, i32) {
    let mut transaction = app.pool.begin().await.expect("begin transaction");
    let backend = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await
        .expect("read backend pid");

    (transaction, backend)
}

async fn close_uncommitted(
    transaction: &mut Transaction<'static, Postgres>,
    course: &TeachingCourse,
) {
    sqlx::query("update teaching_courses set closed_at = now() where id = $1")
        .bind(course.id().as_uuid())
        .execute(&mut **transaction)
        .await
        .expect("close the course");
}

async fn move_student_uncommitted(
    transaction: &mut Transaction<'static, Postgres>,
    student: &User,
    group: &StudentGroup,
) {
    sqlx::query("select id from users where id = $1 for update")
        .bind(student.id().as_uuid())
        .execute(&mut **transaction)
        .await
        .expect("lock the student");
    sqlx::query("update users set student_group_id = $2 where id = $1")
        .bind(student.id().as_uuid())
        .bind(group.id().as_uuid())
        .execute(&mut **transaction)
        .await
        .expect("move the student");
    sqlx::query("delete from teaching_course_students where student_id = $1")
        .bind(student.id().as_uuid())
        .execute(&mut **transaction)
        .await
        .expect("drop the enrollments");
}

#[tokio::test]
async fn creating_an_assignment_in_a_closed_course_is_a_conflict() {
    let app = TestApp::start().await;
    app.mark_course_closed(&app.fixture.course).await;

    app.create_assignment(app.course(), app.teacher())
        .await
        .error(StatusCode::CONFLICT, "course_closed");
}

#[tokio::test]
async fn publishing_in_a_closed_course_is_a_conflict_and_keeps_the_draft() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    app.mark_course_closed(&app.fixture.course).await;

    app.publish_assignment(draft.id().as_uuid(), app.teacher())
        .await
        .error(StatusCode::CONFLICT, "course_closed");

    let stored = db::assignments::find_by_id(&app.pool, draft.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status(), draft.status());
}

#[tokio::test]
async fn the_invite_of_a_closed_course_is_not_issued() {
    let app = TestApp::start().await;
    app.mark_course_closed(&app.fixture.course).await;

    let error = app
        .application
        .courses
        .course_invite(app.teacher_actor(), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::CourseClosed);
}

#[tokio::test]
async fn the_invite_of_a_closed_course_is_not_rotated() {
    let app = TestApp::start().await;
    let course = app.fixture.course.id();
    let generation = db::course_invites::find_generation(&app.pool, course)
        .await
        .unwrap()
        .unwrap();
    app.mark_course_closed(&app.fixture.course).await;

    let error = app
        .application
        .courses
        .rotate_course_invite(app.teacher_actor(), course, generation)
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::CourseClosed);
    assert_eq!(
        db::course_invites::find_generation(&app.pool, course)
            .await
            .unwrap(),
        Some(generation)
    );
}

#[tokio::test]
async fn makeup_cannot_be_configured_in_a_closed_course() {
    let app = TestApp::start().await;
    app.mark_course_closed(&app.fixture.course).await;
    let now = Utc::now();

    let error = app
        .application
        .courses
        .configure_makeup(ConfigureMakeup {
            course_id: app.fixture.course.id(),
            actor: app.teacher_actor(),
            expected: None,
            settings: Some(MakeupSettings {
                until: now + Duration::days(30),
                attempts: 1,
            }),
            decisions: Vec::new(),
            now,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::CourseClosed);
}

#[tokio::test]
async fn a_closed_course_is_not_offered_for_enrollment() {
    let app = TestApp::start().await;
    let open = app.add_subject_course("Algorithms").await;
    let closed = app.add_subject_course("Databases").await;
    app.mark_course_closed(&closed).await;

    let offered = db::enrollments::offered_courses(&app.pool, app.fixture.group.id(), None)
        .await
        .unwrap();

    let ids: Vec<_> = offered.iter().map(|course| course.course_id).collect();
    assert!(ids.contains(&open.id()));
    assert!(!ids.contains(&closed.id()));
}

#[tokio::test]
async fn enrolling_in_a_closed_course_is_invalid_input() {
    let app = TestApp::start().await;
    let closed = app.add_subject_course("Databases").await;
    let student = app.add_unenrolled_student("Late").await;
    app.mark_course_closed(&closed).await;

    let error = app
        .application
        .users
        .enroll(Actor::new(student.id()), &BTreeSet::from([closed.id()]))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_closed_course_leaves_the_working_list_for_the_closed_one() {
    let app = TestApp::start().await;
    let open = app.add_subject_course("Algorithms").await;
    app.mark_course_closed(&app.fixture.course).await;
    let courses = &app.application.courses;

    let working = courses
        .list_for_teacher(app.teacher_actor(), Default::default())
        .await
        .unwrap();
    let finished = courses
        .list_closed_for_teacher(app.teacher_actor(), Default::default())
        .await
        .unwrap();

    let ids = |page: &hw_bot::pagination::Page<hw_bot::app::TeachingCourseDetails>| {
        page.items
            .iter()
            .map(|item| item.course.id())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&working), [open.id()]);
    assert_eq!(ids(&finished), [app.fixture.course.id()]);
}

#[tokio::test]
async fn publishing_waits_for_a_closing_course_and_is_refused() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    let (mut closing, backend) = blocking_transaction(&app).await;
    close_uncommitted(&mut closing, &app.fixture.course).await;

    let (published, ()) = tokio::join!(
        app.application
            .assignments
            .publish(app.teacher_actor(), draft.id(), Utc::now()),
        async {
            wait_for_blocked_connection(&app.pool, backend).await;
            closing.commit().await.expect("commit closing");
        }
    );

    assert_eq!(published.unwrap_err().code(), ErrorCode::CourseClosed);
}

#[tokio::test]
async fn closing_waits_for_a_publication_in_progress() {
    let app = TestApp::start().await;
    let (mut publishing, backend) = blocking_transaction(&app).await;
    sqlx::query("select id from teaching_courses where id = $1 for share")
        .bind(app.course())
        .execute(&mut *publishing)
        .await
        .expect("hold the course like a publication");

    let (closed, ()) = tokio::join!(
        sqlx::query("update teaching_courses set closed_at = now() where id = $1")
            .bind(app.course())
            .execute(&app.pool),
        async {
            wait_for_blocked_connection(&app.pool, backend).await;
            publishing.commit().await.expect("commit publication");
        }
    );

    assert_eq!(closed.expect("close the course").rows_affected(), 1);
}

#[tokio::test]
async fn creating_an_assignment_waits_for_a_closing_course_and_is_refused() {
    let app = TestApp::start().await;
    let (mut closing, backend) = blocking_transaction(&app).await;
    close_uncommitted(&mut closing, &app.fixture.course).await;

    let (created, ()) = tokio::join!(app.create_assignment(app.course(), app.teacher()), async {
        wait_for_blocked_connection(&app.pool, backend).await;
        closing.commit().await.expect("commit closing");
    });

    created.error(StatusCode::CONFLICT, "course_closed");
}

#[tokio::test]
async fn a_submission_waits_for_a_group_change_and_is_judged_by_the_new_group() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let (mut moving, backend) = blocking_transaction(&app).await;
    move_student_uncommitted(&mut moving, &app.fixture.student, &foreign.group).await;

    let (submitted, ()) = tokio::join!(
        app.application.submissions.submit_homework(SubmitHomework {
            assignment_id: app.fixture.assignment.id(),
            actor: app.student_actor(),
            content: "pull/1".to_owned(),
            submitted_at: Utc::now(),
            allow_makeup: false,
        }),
        async {
            wait_for_blocked_connection(&app.pool, backend).await;
            moving.commit().await.expect("commit the group change");
        }
    );

    assert_eq!(submitted.unwrap_err().code(), ErrorCode::PermissionDenied);
    let stored: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn an_enrollment_waits_for_a_group_change_and_leaves_no_old_record() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let student = app.add_unenrolled_student("Moving").await;
    let (mut moving, backend) = blocking_transaction(&app).await;
    move_student_uncommitted(&mut moving, &student, &foreign.group).await;

    let courses = BTreeSet::from([app.fixture.course.id()]);
    let (enrolled, ()) = tokio::join!(
        app.application
            .users
            .enroll(Actor::new(student.id()), &courses),
        async {
            wait_for_blocked_connection(&app.pool, backend).await;
            moving.commit().await.expect("commit the group change");
        }
    );

    assert_eq!(enrolled.unwrap_err().code(), ErrorCode::InvalidInput);
    let records: i64 =
        sqlx::query_scalar("select count(*) from teaching_course_students where student_id = $1")
            .bind(student.id().as_uuid())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(records, 0);
}
