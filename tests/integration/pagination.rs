use hw_bot::app::{Actor, ErrorCode};
use hw_bot::bot::{callbacks, texts};
use hw_bot::domain::{Assignment, UserId};
use hw_bot::test_support::ReviewShortcuts;
use hw_bot::{
    db,
    pagination::{PAGE_SIZE, PageRequest},
};

use crate::harness::telegram::{cb, start_bot, stop_bot};
use crate::harness::telegram_scenario::TelegramScenario;
use crate::harness::{TestApp, insert_assignment};

async fn add_published_assignments(app: &TestApp, count: usize) {
    for index in 0..count {
        app.add_published_assignment(
            &app.fixture.course,
            &app.fixture.teacher,
            &format!("Assignment {index}"),
        )
        .await;
    }
}

#[tokio::test]
async fn telegram_opens_the_next_assignment_page_using_a_real_navigation_button() {
    let app = TestApp::start().await;
    add_published_assignments(&app, PAGE_SIZE).await;
    let second = app
        .application
        .assignments
        .list_for_student(app.student_actor(), PageRequest::new(1))
        .await
        .unwrap();
    let id = second.items[0].assignment.id();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    telegram.message(app.student(), "/tasks");
    let forward = telegram
        .button_with_text(app.student(), texts::BUTTON_NEXT_PAGE)
        .await;
    telegram.click(app.student(), &forward);
    let assignment = telegram
        .button_after(
            app.student(),
            &cb(callbacks::ASSIGNMENT_PREFIX, id.as_uuid()),
            forward.message_id,
        )
        .await;
    let back = telegram
        .button_with_text(app.student(), texts::BUTTON_PREVIOUS_PAGE)
        .await;
    stop_bot(cancellation, &mut task).await;

    assert!(assignment.message_id > forward.message_id);
    assert_eq!(back.message_id, assignment.message_id);
}

#[tokio::test]
async fn assignment_pages_are_bounded_disjoint_and_exhaustive() {
    let app = TestApp::start().await;
    add_published_assignments(&app, PAGE_SIZE).await;

    let first = app.get_as("/me/assignments", app.student()).await;
    assert_eq!(first.headers()["x-next-page"], "1");
    let first = first.ok();
    let second = app.get_as("/me/assignments?page=1", app.student()).await;
    assert!(second.headers().get("x-next-page").is_none());
    let second = second.ok();

    assert_eq!(first.as_array().unwrap().len(), PAGE_SIZE);
    assert_eq!(second.as_array().unwrap().len(), 1);
    assert!(
        first
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["assignment"]["id"] != second[0]["assignment"]["id"])
    );
}

#[tokio::test]
async fn profile_statistics_count_assignments_beyond_the_first_page() {
    let app = TestApp::start().await;
    add_published_assignments(&app, PAGE_SIZE).await;

    let stats = app
        .application
        .users
        .statistics(app.student_actor())
        .await
        .unwrap();

    assert_eq!(stats.assignments as usize, PAGE_SIZE + 1);
}

#[tokio::test]
async fn grade_totals_cover_all_pages() {
    let app = TestApp::start().await;
    for _ in 0..PAGE_SIZE + 1 {
        app.add_scored_assignment(5).await;
    }
    let grades = |page| {
        app.application.courses.student_grades(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.student.id(),
            page,
        )
    };

    let first = grades(PageRequest::default()).await.unwrap();
    let second = grades(PageRequest::new(1)).await.unwrap();

    assert_eq!(first.grades.len(), PAGE_SIZE);
    assert_eq!(first.next_grade_page, Some(1));
    assert_eq!(second.grades.len(), 1);
    assert_eq!(second.next_grade_page, None);
    assert_eq!(first.max_total(), (PAGE_SIZE as u32 + 1) * 5);
    assert_eq!(first.max_total(), second.max_total());
    assert_eq!(first.total(), 0);
    assert!(first.grades.iter().all(|grade| grade.score.is_none()));
}

#[tokio::test]
async fn a_teacher_of_another_course_cannot_read_student_grades() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    let error = app
        .application
        .courses
        .student_grades(
            Actor::new(outsider.id()),
            app.fixture.course.id(),
            app.fixture.student.id(),
            PageRequest::default(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn grades_of_an_unknown_student_are_not_found() {
    let app = TestApp::start().await;

    let error = app
        .application
        .courses
        .student_grades(
            app.teacher_actor(),
            app.fixture.course.id(),
            UserId::new(),
            PageRequest::default(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NotFound);
}

async fn rejected_attempts(app: &TestApp, count: usize) -> Assignment {
    let assignment = hw_bot::test_support::assignment(&app.fixture.course, &app.fixture.teacher)
        .max_attempts(20)
        .published()
        .build();
    insert_assignment(&app.pool, &assignment).await;
    for index in 0..count {
        let mut submission = hw_bot::test_support::submission(&assignment, &app.fixture.student)
            .history(hw_bot::test_support::history().used(index as u16).build())
            .build();
        submission
            .reject(
                &assignment,
                &app.fixture.course,
                &app.fixture.teacher,
                "Again",
                chrono::Utc::now(),
            )
            .unwrap();
        db::submissions::insert(&app.pool, &submission)
            .await
            .unwrap();
    }

    assignment
}

#[tokio::test]
async fn the_card_carries_only_the_latest_attempt() {
    let app = TestApp::start().await;
    let assignment = rejected_attempts(&app, PAGE_SIZE + 1).await;

    let details = app
        .application
        .assignments
        .find_for_student(app.student_actor(), assignment.id())
        .await
        .unwrap();

    let latest = details
        .latest_submission
        .expect("the newest attempt is on the card");
    assert_eq!(latest.attempt_number() as usize, PAGE_SIZE + 1);
}

#[tokio::test]
async fn submission_history_is_paged_newest_first() {
    let app = TestApp::start().await;
    let assignment = rejected_attempts(&app, PAGE_SIZE + 1).await;
    let history = |page| {
        app.application
            .assignments
            .submission_history(app.student_actor(), assignment.id(), page)
    };

    let first = history(PageRequest::default()).await.unwrap().submissions;
    let last = history(PageRequest::new(1)).await.unwrap().submissions;

    assert_eq!(first.items.len(), PAGE_SIZE);
    assert_eq!(first.next_page, Some(1));
    assert!(
        first
            .items
            .windows(2)
            .all(|pair| pair[0].attempt_number() > pair[1].attempt_number())
    );
    assert_eq!(last.items[0].attempt_number(), 1);
    assert_eq!(last.next_page, None);
}
