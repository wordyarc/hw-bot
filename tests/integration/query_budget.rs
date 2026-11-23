use std::time::Duration;

use chrono::Utc;
use hw_bot::pagination::PageRequest;
use hw_bot::telegram::{DeliveryOutcome, NotificationWorker, NotificationWorkerSettings, new_bot};

use crate::harness::query_log::QueryLog;
use crate::harness::{TestApp, insert_assignment, wall_clock_assignment};

async fn add_submitted_assignments(app: &TestApp, count: usize) {
    for _ in 0..count {
        let assignment = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
            .published()
            .build();
        insert_assignment(&app.pool, &assignment).await;
        app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created();
    }
}

fn assert_same_count(few: &[String], many: &[String]) {
    assert!(!few.is_empty(), "no statements were captured");
    assert_eq!(many.len(), few.len(), "{many:#?}");
}

#[tokio::test]
async fn listing_student_assignments_does_not_query_per_assignment() {
    let app = TestApp::start().await;
    let (log, _capture) = QueryLog::capture();
    let list = || {
        app.application
            .assignments
            .list_for_student(app.student_actor(), PageRequest::new(0))
    };
    let (listed, one) = log.during(list()).await;
    assert_eq!(listed.unwrap().items.len(), 1);
    add_submitted_assignments(&app, 6).await;

    let (listed, seven) = log.during(list()).await;

    assert_eq!(listed.unwrap().items.len(), 7);
    assert_same_count(&one, &seven);
}

#[tokio::test]
async fn listing_the_review_queue_does_not_query_per_submission() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;
    let (log, _capture) = QueryLog::capture();
    let list = || {
        app.application
            .reviews
            .list_page(app.teacher_actor(), PageRequest::new(0))
    };
    let (listed, one) = log.during(list()).await;
    assert_eq!(listed.unwrap().items.len(), 1);
    add_submitted_assignments(&app, 6).await;

    let (listed, seven) = log.during(list()).await;

    assert_eq!(listed.unwrap().items.len(), 7);
    assert_same_count(&one, &seven);
}

#[tokio::test]
async fn listing_queue_places_does_not_query_per_submission() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;
    let (log, _capture) = QueryLog::capture();
    let list = || {
        app.application
            .submissions
            .queue_places(app.student_actor(), PageRequest::new(0))
    };
    let (listed, one) = log.during(list()).await;
    assert_eq!(listed.unwrap().items.len(), 1);
    add_submitted_assignments(&app, 6).await;

    let (listed, seven) = log.during(list()).await;

    assert_eq!(listed.unwrap().items.len(), 7);
    assert_same_count(&one, &seven);
}

#[tokio::test]
async fn listing_teacher_courses_does_not_query_per_course() {
    let app = TestApp::start().await;
    let (log, _capture) = QueryLog::capture();
    let list = || {
        app.application
            .courses
            .list_for_teacher(app.teacher_actor(), PageRequest::new(0))
    };
    let (listed, one) = log.during(list()).await;
    assert_eq!(listed.unwrap().items.len(), 1);
    for name in ["Algebra", "Biology", "Chemistry"] {
        app.add_subject_course(name).await;
    }

    let (listed, four) = log.during(list()).await;

    assert_eq!(listed.unwrap().items.len(), 4);
    assert_same_count(&one, &four);
}

#[tokio::test]
async fn reading_the_grade_sheet_does_not_query_per_student_or_assignment() {
    let app = TestApp::start().await;
    let (log, _capture) = QueryLog::capture();
    let read = || {
        app.application
            .courses
            .grade_sheet(app.teacher_actor(), app.fixture.course.id())
    };
    let (sheet, small) = log.during(read()).await;
    assert_eq!(sheet.unwrap().unwrap().groups[0].students.len(), 1);
    add_submitted_assignments(&app, 6).await;
    for name in ["Anna", "Boris", "Vera"] {
        app.add_student(name).await;
    }

    let (sheet, large) = log.during(read()).await;

    let sheet = sheet.unwrap().unwrap();
    assert_eq!(sheet.groups[0].students.len(), 4);
    assert_eq!(sheet.groups[0].columns.len(), 7);
    assert_same_count(&small, &large);
}

#[tokio::test]
async fn an_idle_notification_poll_is_a_single_statement() {
    let app = TestApp::start().await;
    let settings = NotificationWorkerSettings {
        request_timeout: Duration::from_secs(1),
        poll_interval: Duration::from_millis(10),
        lease_duration: Duration::from_secs(25),
        max_attempts: 3,
        retry_initial: Duration::from_secs(1),
        retry_max: Duration::from_secs(10),
    };
    let telegram = new_bot("test-token", "http://127.0.0.1:9", settings.request_timeout).unwrap();
    let worker = NotificationWorker::new(app.pool.clone(), telegram, settings);
    let (log, _capture) = QueryLog::capture();

    let (outcome, statements) = log.during(worker.deliver_one(Utc::now())).await;

    assert_eq!(outcome.unwrap(), DeliveryOutcome::Idle);
    assert_eq!(statements.len(), 1, "{statements:#?}");
}
