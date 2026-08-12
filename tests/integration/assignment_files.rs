use hw_bot::app::{
    Actor, AddAssignmentFile, ApplicationContainer, DeleteAssignmentFile, ErrorCode, FilePolicy,
};
use hw_bot::domain::{FilePresentation, StoredFileOwner};

use crate::harness::{TestApp, test_file_policy};

const DOCUMENT: &[u8] = include_bytes!("fixtures/assignment-note.txt");
const IMAGE: &[u8] = include_bytes!("fixtures/pixel.ppm");

fn application(app: &TestApp) -> ApplicationContainer {
    ApplicationContainer::new(app.pool.clone(), test_file_policy())
}

async fn card(app: &TestApp, application: &ApplicationContainer) -> hw_bot::app::AssignmentFiles {
    application
        .files
        .list_assignment_files_for_teacher(
            Actor::new(app.fixture.teacher.id()),
            app.fixture.assignment.id(),
        )
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
            actor: Actor::new(app.fixture.teacher.id()),
            expected_revision: revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
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
            Actor::new(app.fixture.teacher.id()),
            app.fixture.assignment.id(),
            metadata.id(),
        )
        .await
        .unwrap();

    assert_eq!(opened.content, DOCUMENT);
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
            actor: Actor::new(app.fixture.teacher.id()),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
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
            actor: Actor::new(app.fixture.teacher.id()),
            expected_revision: added.revision,
            presentation: FilePresentation::Image,
            filename: "pixel.ppm".to_owned(),
            mime_type: Some("image/x-portable-pixmap".to_owned()),
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
        actor: Actor::new(app.fixture.teacher.id()),
        expected_revision: initial.revision,
        presentation: FilePresentation::Document,
        filename: filename.to_owned(),
        mime_type: None,
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
async fn published_file_deletion_requires_confirmation() {
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

    let error = application
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            file_id,
            actor: Actor::new(app.fixture.teacher.id()),
            expected_revision: added.revision,
            confirmed: false,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::ConfirmationRequired);
    let current = card(&app, &application).await;
    assert_eq!(current.revision, added.revision);
    assert_eq!(current.files.len(), 1);
}

#[tokio::test]
async fn confirmed_deletion_removes_file_and_advances_revision() {
    let app = TestApp::start().await;
    let application = application(&app);
    let added = add(
        &app,
        &application,
        card(&app, &application).await.revision,
        DOCUMENT,
    )
    .await;

    let deleted = application
        .files
        .delete_assignment_file(DeleteAssignmentFile {
            assignment_id: app.fixture.assignment.id(),
            file_id: added.files[0].id(),
            actor: Actor::new(app.fixture.teacher.id()),
            expected_revision: added.revision,
            confirmed: true,
        })
        .await
        .unwrap();

    assert!(deleted.files.is_empty());
    assert_eq!(deleted.revision, added.revision + 1);
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
            actor: Actor::new(outsider.id()),
            expected_revision: initial.revision,
            presentation: FilePresentation::Document,
            filename: "assignment-note.txt".to_owned(),
            mime_type: None,
            content: DOCUMENT.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(card(&app, &application).await.files.is_empty());
}
