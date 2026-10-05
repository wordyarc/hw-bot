use hw_bot::app::{
    AddAssignmentFile, ApplicationContainer, ChangeDeadline, DeleteAssignmentFile, ErrorCode,
    FilePolicy,
};
use hw_bot::domain::{AssignmentStatus, FilePresentation, StoredFileOwner};

use crate::harness::{TestApp, test_file_policy};

const DOCUMENT: &[u8] = include_bytes!("fixtures/assignment-note.txt");
const IMAGE: &[u8] = include_bytes!("fixtures/pixel.ppm");

fn application(app: &TestApp) -> ApplicationContainer {
    app.application.clone()
}

async fn card(app: &TestApp, application: &ApplicationContainer) -> hw_bot::app::AssignmentFiles {
    application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), app.fixture.assignment.id())
        .await
        .unwrap()
}

async fn add(
    app: &TestApp,
    application: &ApplicationContainer,
    revision: i64,
    content: &[u8],
) -> hw_bot::app::AssignmentFiles {
    application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            expected_revision: revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: content.to_vec(),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn assignment_without_files_has_an_empty_attachment_list() {
    let app = TestApp::start().await;
    let application = application(&app);

    let card = card(&app, &application).await;

    assert!(card.files.is_empty());
}

#[tokio::test]
async fn stored_file_keeps_exact_fixture_bytes_and_computed_metadata() {
    let app = TestApp::start().await;
    let application = application(&app);
    let initial = card(&app, &application).await;

    let added = add(&app, &application, initial.revision, DOCUMENT).await;
    let metadata = added.files.first().unwrap();
    let opened = application
        .files
        .open_assignment_file_for_teacher(
            app.teacher_actor(),
            app.fixture.assignment.id(),
            metadata.id(),
        )
        .await
        .unwrap();

    assert_eq!(
        application.files.read_content(&opened).await.unwrap(),
        DOCUMENT
    );
    assert_eq!(metadata.size_bytes(), DOCUMENT.len() as u64);
    assert_eq!(
        metadata.sha256(),
        [
            0x52, 0xab, 0x7a, 0x88, 0x04, 0xd0, 0xee, 0x36, 0xf3, 0x6e, 0xc8, 0x8f, 0xee, 0xfa,
            0xfd, 0x72, 0x7c, 0x9d, 0xa2, 0xd2, 0x96, 0x31, 0xe1, 0xeb, 0x03, 0xdf, 0x5a, 0x19,
            0xde, 0x76, 0x13, 0x8b,
        ]
    );
    assert_eq!(
        metadata.owner(),
        StoredFileOwner::Assignment(app.fixture.assignment.id())
    );
}

#[tokio::test]
async fn student_opens_a_published_assignment_file() {
    let app = TestApp::start().await;
    let application = application(&app);
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;

    let opened = application
        .files
        .open_assignment_file_for_student(
            app.student_actor(),
            app.fixture.assignment.id(),
            added.files[0].id(),
        )
        .await
        .unwrap();

    assert_eq!(
        application.files.read_content(&opened).await.unwrap(),
        DOCUMENT
    );
}

#[tokio::test]
async fn student_keeps_access_to_an_archived_assignment_file() {
    let app = TestApp::start().await;
    let application = application(&app);
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;
    application
        .assignments
        .archive(app.fixture.assignment.id(), app.teacher_actor())
        .await
        .unwrap();

    let opened = application
        .files
        .open_assignment_file_for_student(
            app.student_actor(),
            app.fixture.assignment.id(),
            added.files[0].id(),
        )
        .await
        .unwrap();

    assert_eq!(
        application.files.read_content(&opened).await.unwrap(),
        DOCUMENT
    );
}

#[tokio::test]
async fn a_draft_assignment_file_does_not_exist_for_a_student() {
    let app = TestApp::start().await;
    let application = application(&app);
    let draft = app.add_draft_assignment().await;
    let card = application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), draft.id())
        .await
        .unwrap();
    let added = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: draft.id(),
            actor: app.teacher_actor(),
            expected_revision: card.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap();

    let error = application
        .files
        .open_assignment_file_for_student(app.student_actor(), draft.id(), added.files[0].id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn a_never_published_archived_file_does_not_exist_for_a_student() {
    let app = TestApp::start().await;
    let application = application(&app);
    let draft = app.add_draft_assignment().await;
    let card = application
        .files
        .list_assignment_files_for_teacher(app.teacher_actor(), draft.id())
        .await
        .unwrap();
    let added = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: draft.id(),
            actor: app.teacher_actor(),
            expected_revision: card.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap();
    application
        .assignments
        .archive(draft.id(), app.teacher_actor())
        .await
        .unwrap();

    let error = application
        .files
        .open_assignment_file_for_student(app.student_actor(), draft.id(), added.files[0].id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn another_group_cannot_open_an_assignment_file() {
    let app = TestApp::start().await;
    let application = application(&app);
    let outsider = app.add_outsider_student().await;
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;

    let error = application
        .files
        .open_assignment_file_for_student(
            hw_bot::app::Actor::new(outsider.id()),
            app.fixture.assignment.id(),
            added.files[0].id(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn file_size_limit_is_application_configuration() {
    let app = TestApp::start().await;
    let application = ApplicationContainer::new(
        app.pool.clone(),
        FilePolicy {
            max_file_size_bytes: DOCUMENT.len() - 1,
            ..test_file_policy()
        },
    );
    let initial = card(&app, &application).await;

    let error = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::FileTooLarge);
    assert_eq!(card(&app, &application).await.revision, initial.revision);
}

#[tokio::test]
async fn assignment_file_count_limit_is_application_configuration() {
    let app = TestApp::start().await;
    let application = ApplicationContainer::new(
        app.pool.clone(),
        FilePolicy {
            max_files_per_assignment: 1,
            ..test_file_policy()
        },
    );
    let initial = card(&app, &application).await;
    let added = add(&app, &application, initial.revision, DOCUMENT).await;

    let error = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            expected_revision: added.revision,
            presentation: FilePresentation::Image,
            filename: "pixel.ppm".to_owned(),
            mime_type: Some("image/x-portable-pixmap".to_owned()),
            telegram_file_id: None,
            content: IMAGE.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::FileLimitReached);
    assert_eq!(card(&app, &application).await.revision, added.revision);
}

#[tokio::test]
async fn concurrent_additions_with_one_revision_have_one_winner() {
    let app = TestApp::start().await;
    let application = application(&app);
    let initial = card(&app, &application).await;
    let command = |filename: &str| AddAssignmentFile {
        assignment_id: app.fixture.assignment.id(),
        actor: app.teacher_actor(),
        expected_revision: initial.revision,
        presentation: FilePresentation::Document,
        filename: filename.to_owned(),
        mime_type: None,
        telegram_file_id: None,
        content: DOCUMENT.to_vec(),
    };

    let (first, second) = tokio::join!(
        application.files.add_assignment_file(command("one.txt")),
        application.files.add_assignment_file(command("two.txt")),
    );

    let results = [first, second];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .unwrap()
            .code(),
        ErrorCode::ConcurrentModification
    );
    assert_eq!(card(&app, &application).await.files.len(), 1);
}

#[tokio::test]
async fn app_deletes_a_published_file_without_transport_confirmation_state() {
    let app = TestApp::start().await;
    let application = application(&app);
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;
    let file_id = added.files[0].id();

    let deleted = application
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            file_id,
            actor: app.teacher_actor(),
            expected_revision: added.revision,
        })
        .await
        .unwrap();

    assert_eq!(added.assignment.status(), AssignmentStatus::Published);
    assert_eq!(deleted.revision, added.revision + 1);
    assert!(deleted.files.is_empty());
}

#[tokio::test]
async fn unrelated_teacher_cannot_manage_assignment_files() {
    let app = TestApp::start().await;
    let application = application(&app);
    let outsider = app.add_outsider_teacher("Outsider").await;
    let initial = card(&app, &application).await;

    let error = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: hw_bot::app::Actor::new(outsider.id()),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: None,
            telegram_file_id: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(card(&app, &application).await.files.is_empty());
}

#[tokio::test]
async fn unrelated_teacher_cannot_list_open_or_delete_assignment_files() {
    let app = TestApp::start().await;
    let application = application(&app);
    let outsider = app.add_outsider_teacher("Outsider").await;
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;
    let outsider = hw_bot::app::Actor::new(outsider.id());

    let list = application
        .files
        .list_assignment_files_for_teacher(outsider, app.fixture.assignment.id())
        .await
        .unwrap_err();
    let open = application
        .files
        .open_assignment_file_for_teacher(
            outsider,
            app.fixture.assignment.id(),
            added.files[0].id(),
        )
        .await
        .unwrap_err();
    let delete = application
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            file_id: added.files[0].id(),
            actor: outsider,
            expected_revision: added.revision,
        })
        .await
        .unwrap_err();

    assert_eq!(list.code(), ErrorCode::PermissionDenied);
    assert_eq!(open.code(), ErrorCode::PermissionDenied);
    assert_eq!(delete.code(), ErrorCode::PermissionDenied);
    assert_eq!(card(&app, &application).await.files.len(), 1);
}

#[tokio::test]
async fn assignment_change_invalidates_an_old_file_card() {
    let app = TestApp::start().await;
    let application = application(&app);
    let initial = card(&app, &application).await;

    application
        .assignments
        .change_deadline(ChangeDeadline {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            deadline_at: app.fixture.assignment.deadline_at() + chrono::Duration::hours(1),
            expected_revision: None,
        })
        .await
        .unwrap();
    let error = application
        .files
        .add_assignment_file(AddAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            actor: app.teacher_actor(),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            telegram_file_id: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::ConcurrentModification);
    assert!(card(&app, &application).await.files.is_empty());
}
