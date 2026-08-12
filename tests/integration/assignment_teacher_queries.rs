use hw_bot::app::{Actor, ApplicationContainer, ErrorCode};

use crate::harness::{TestApp, test_file_policy};

#[tokio::test]
async fn course_teacher_can_open_a_draft_assignment_card() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let assignment = application
        .assignments
        .find_for_teacher(Actor::new(app.fixture.teacher.id()), draft.id())
        .await
        .unwrap();

    assert_eq!(assignment, draft);
}

#[tokio::test]
async fn unrelated_teacher_cannot_open_an_assignment_card() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let error = application
        .assignments
        .find_for_teacher(Actor::new(outsider.id()), app.fixture.assignment.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn unrelated_teacher_cannot_list_assignment_students() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let error = application
        .assignments
        .list_students_for_assignment(Actor::new(outsider.id()), app.fixture.assignment.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn assignment_student_list_includes_only_its_group_with_default_progress() {
    let app = TestApp::start().await;
    let second = app.add_student("Second student").await;
    let foreign = app.add_foreign_course().await;
    let foreign_student = app
        .add_student_to_group("Foreign student", &foreign.group)
        .await;
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let students = application
        .assignments
        .list_students_for_assignment(
            Actor::new(app.fixture.teacher.id()),
            app.fixture.assignment.id(),
        )
        .await
        .unwrap();

    let ids: Vec<_> = students.iter().map(|item| item.student.id()).collect();
    assert!(ids.contains(&app.fixture.student.id()));
    assert!(ids.contains(&second.id()));
    assert!(!ids.contains(&foreign_student.id()));
    assert!(
        students
            .iter()
            .all(|item| item.history.used_attempts() == 0)
    );
    assert!(
        students
            .iter()
            .all(|item| item.history.extra_attempts() == 0)
    );
}

#[tokio::test]
async fn assignment_student_list_contains_submission_and_extra_attempt_facts() {
    let app = TestApp::start().await;
    app.queued_submission(app.student(), "pull/1").await;
    app.grant_extra_attempts(
        app.assignment(),
        app.fixture.student.id().as_uuid(),
        app.teacher(),
        2,
    )
    .await
    .ok();
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let students = application
        .assignments
        .list_students_for_assignment(
            Actor::new(app.fixture.teacher.id()),
            app.fixture.assignment.id(),
        )
        .await
        .unwrap();
    let student = students
        .iter()
        .find(|item| item.student.id() == app.fixture.student.id())
        .unwrap();

    assert_eq!(student.history.used_attempts(), 0);
    assert_eq!(student.history.extra_attempts(), 2);
    assert!(student.history.has_pending_submission());
    assert!(!student.history.has_accepted_submission());
}

#[tokio::test]
async fn assignment_student_list_contains_accepted_submission_facts() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "good")
        .await
        .ok();
    let application = ApplicationContainer::new(app.pool.clone(), test_file_policy());

    let students = application
        .assignments
        .list_students_for_assignment(
            Actor::new(app.fixture.teacher.id()),
            app.fixture.assignment.id(),
        )
        .await
        .unwrap();
    let student = students
        .iter()
        .find(|item| item.student.id() == app.fixture.student.id())
        .unwrap();

    assert_eq!(student.history.used_attempts(), 1);
    assert!(student.history.has_accepted_submission());
    assert!(!student.history.has_pending_submission());
}
