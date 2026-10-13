use chrono::{Duration, Utc};
use hw_bot::app::Actor;
use hw_bot::db;
use hw_bot::domain::{Assignment, User};
use hw_bot::test_support::{ReviewShortcuts, submission};

use crate::harness::TestApp;

#[tokio::test]
async fn a_student_sees_their_place_in_the_submit_order() {
    let app = TestApp::start().await;
    let first = app.add_student("First").await;
    let second = app.add_student("Second").await;

    submit(&app, &app.fixture.assignment, &first, 3).await;
    submit(&app, &app.fixture.assignment, &second, 2).await;
    submit(&app, &app.fixture.assignment, &app.fixture.student, 1).await;

    let places = app
        .application
        .submissions
        .pending_queue(Actor::new(first.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(places.len(), 1);
    assert_eq!(places[0].assignment_id, app.fixture.assignment.id());
    assert_eq!(places[0].subject_name, "Backend development");
    assert_eq!(places[0].position, 1, "submitted first");
    assert_eq!(places[0].total, 3);

    let last = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(last[0].position, 3, "submitted last");
}

#[tokio::test]
async fn a_reviewed_submission_leaves_the_queue() {
    let app = TestApp::start().await;
    let student = app.add_student("Ahead").await;
    submit(&app, &app.fixture.assignment, &student, 2).await;
    submit(&app, &app.fixture.assignment, &app.fixture.student, 1).await;

    let mut ahead = db::submissions::list_for_student(
        &app.pool,
        app.fixture.assignment.id(),
        student.id(),
        Default::default(),
    )
    .await
    .expect("the submission is stored")
    .remove(0);
    ahead
        .accept(
            &app.fixture.assignment,
            &app.fixture.course,
            &app.fixture.teacher,
            None,
            "Принято",
            Utc::now(),
        )
        .expect("a course teacher may accept");
    db::submissions::save_review_result(&app.pool, &ahead)
        .await
        .expect("the review is stored");

    let places = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(places[0].position, 1, "the reviewed one no longer counts");
    assert_eq!(places[0].total, 1);
}

/// Сдача другого курса учитывается перед студентом, если её проверяет тот же преподаватель.
/// Работа из очереди чужого преподавателя на позицию не влияет.
#[tokio::test]
async fn the_queue_spans_every_course_of_the_same_teachers() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;
    let foreign_student = app.add_student_to_group("Foreign", &foreign.group).await;
    let foreign_assignment = app
        .add_published_assignment(&foreign.course, &foreign.teacher, "Foreign work")
        .await;

    submit(&app, &foreign_assignment, &foreign_student, 2).await;
    submit(&app, &app.fixture.assignment, &app.fixture.student, 1).await;

    let alone = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(alone[0].total, 1, "a foreign teacher reviews their own");

    app.attach_course_teacher(&foreign.course, &app.fixture.teacher)
        .await;

    let shared = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(shared[0].total, 2, "one reviewer now sees both courses");
    assert_eq!(
        shared[0].position, 2,
        "the foreign work was submitted first"
    );
}

#[tokio::test]
async fn earlier_sandbox_submissions_do_not_delay_a_real_one() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let trainee = app.add_student("Trainee").await;
    submit(&app, &trial, &trainee, 2).await;
    submit(&app, &app.fixture.assignment, &app.fixture.student, 1).await;

    let places = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(places[0].position, 1);
    assert_eq!(places[0].total, 1);
}

#[tokio::test]
async fn a_sandbox_submission_is_placed_among_sandbox_submissions_only() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let trainee = app.add_student("Trainee").await;
    let ahead = app.add_student("Ahead").await;
    submit(&app, &trial, &trainee, 3).await;
    submit(&app, &app.fixture.assignment, &ahead, 2).await;
    submit(&app, &trial, &app.fixture.student, 1).await;

    let places = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(places[0].assignment_id, trial.id());
    assert_eq!(places[0].position, 2, "only the earlier trial is ahead");
    assert_eq!(places[0].total, 2);
}

#[tokio::test]
async fn a_course_without_a_reviewer_reports_no_position() {
    let app = TestApp::start().await;
    app.remove_course_teacher(&app.fixture.teacher).await;
    submit(&app, &app.fixture.assignment, &app.fixture.student, 1).await;

    let places = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.student.id()), Default::default())
        .await
        .expect("the queue is readable")
        .items;

    assert_eq!(places.len(), 1, "the submission itself is still visible");
    assert_eq!(places[0].position, 0);
    assert_eq!(places[0].total, 0);
}

#[tokio::test]
async fn a_teacher_has_no_student_queue() {
    let app = TestApp::start().await;

    let error = app
        .application
        .submissions
        .pending_queue(Actor::new(app.fixture.teacher.id()), Default::default())
        .await
        .expect_err("a teacher reads the review queue, not the student one");

    assert_eq!(error.code(), hw_bot::app::ErrorCode::PermissionDenied);
}

async fn submit(app: &TestApp, assignment: &Assignment, student: &User, minutes_ago: i64) {
    let submission = submission(assignment, student)
        .submitted_at(Utc::now() - Duration::minutes(minutes_ago))
        .build();

    db::submissions::insert(&app.pool, &submission)
        .await
        .expect("insert submission");
}
