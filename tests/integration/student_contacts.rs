use hw_bot::app::{Actor, AppResult, ErrorCode, StudentContact};
use hw_bot::domain::UserId;

use crate::harness::TestApp;

async fn contact(app: &TestApp, actor: Actor, student: UserId) -> AppResult<StudentContact> {
    app.application
        .courses
        .student_contact(actor, app.fixture.course.id(), student)
        .await
}

#[tokio::test]
async fn teacher_reads_registered_identity_and_group_of_own_student() {
    let app = TestApp::start().await;
    let student = &app.fixture.student;

    let result = contact(&app, app.teacher_actor(), student.id())
        .await
        .unwrap();

    assert_eq!(result.student_id, student.id());
    assert_eq!(result.telegram_user_id, student.telegram_user_id());
    assert_eq!(result.display_name, student.display_name().as_str());
    assert_eq!(result.group_code, app.fixture.group.code().as_str());
}

#[tokio::test]
async fn admin_reads_contact_without_course_assignment() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    let result = contact(&app, Actor::new(admin.id()), app.fixture.student.id())
        .await
        .unwrap();

    assert_eq!(result.student_id, app.fixture.student.id());
}

#[tokio::test]
async fn foreign_teacher_is_denied_before_resolving_the_recipient() {
    let app = TestApp::start().await;
    let teacher = app.add_outsider_teacher("Outsider").await;

    for student in [app.fixture.student.id(), UserId::new()] {
        let error = contact(&app, Actor::new(teacher.id()), student)
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::PermissionDenied);
    }
}

#[tokio::test]
async fn student_cannot_read_even_their_own_contact() {
    let app = TestApp::start().await;

    let error = contact(&app, app.student_actor(), app.fixture.student.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn missing_recipient_is_not_found_for_teacher_and_admin() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    for actor in [app.teacher_actor(), Actor::new(admin.id())] {
        let error = contact(&app, actor, UserId::new()).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn staff_recipient_is_not_found_for_teacher_and_admin() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    for actor in [app.teacher_actor(), Actor::new(admin.id())] {
        let error = contact(&app, actor, app.fixture.teacher.id())
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn student_from_another_group_is_not_found_even_for_admin() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;
    let outsider = app.add_outsider_student().await;

    for actor in [app.teacher_actor(), Actor::new(admin.id())] {
        let error = contact(&app, actor, outsider.id()).await.unwrap_err();
        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn contact_does_not_require_submissions_or_graded_assignments() {
    let app = TestApp::start().await;
    let course = app.add_subject_course("Empty course").await;

    let result = app
        .application
        .courses
        .student_contact(app.teacher_actor(), course.id(), app.fixture.student.id())
        .await
        .unwrap();

    assert_eq!(result.student_id, app.fixture.student.id());
}

#[tokio::test]
async fn contact_remains_available_after_review() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "Homework").await;
    app.review(submission, app.teacher(), "accept", "Accepted")
        .await
        .ok();

    assert!(
        contact(&app, app.teacher_actor(), app.fixture.student.id())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn contact_remains_available_after_archiving() {
    let app = TestApp::start().await;
    app.archive_assignment(app.assignment(), app.teacher())
        .await
        .ok();

    assert!(
        contact(&app, app.teacher_actor(), app.fixture.student.id())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn access_is_checked_again_after_teacher_assignment_is_revoked() {
    let app = TestApp::start().await;
    app.add_course_teacher("Remaining teacher").await;
    contact(&app, app.teacher_actor(), app.fixture.student.id())
        .await
        .unwrap();

    app.remove_course_teacher(&app.fixture.teacher).await;

    let error = contact(&app, app.teacher_actor(), app.fixture.student.id())
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}
