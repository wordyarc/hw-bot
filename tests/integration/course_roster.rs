use chrono::{Duration, Utc};
use hw_bot::app::{Actor, AppResult, CourseRoster, ErrorCode, RosterGroup};
use hw_bot::db;
use hw_bot::domain::{Grading, StudentGroup, TeachingCourseId};
use hw_bot::pagination::{PageRequest, ROSTER_PAGE_SIZE};
use hw_bot::test_support::{
    GROUP_CODE, assignment as assignment_of, student_group, submission as submission_of,
};

use crate::harness::{TestApp, insert_assignment, out_of, submission_id, wall_clock_assignment};

async fn read_roster(
    app: &TestApp,
    actor: Actor,
    course_id: TeachingCourseId,
    page: u32,
) -> AppResult<CourseRoster> {
    app.application
        .courses
        .roster(actor, course_id, PageRequest::sized(page, ROSTER_PAGE_SIZE))
        .await
}

async fn roster(app: &TestApp) -> Vec<RosterGroup> {
    read_roster(app, app.teacher_actor(), app.fixture.course.id(), 0)
        .await
        .expect("read the roster")
        .groups
}

async fn add_course_group(app: &TestApp, code: &str) -> StudentGroup {
    let group = student_group(code);
    db::academic::insert_student_group(&app.pool, &group)
        .await
        .expect("insert student group");
    db::academic::add_teaching_course_group(&app.pool, app.fixture.course.id(), group.id())
        .await
        .expect("add the group to the course");

    group
}

type Section<'a> = (&'a str, u32, Vec<(u32, &'a str)>);

fn sections(groups: &[RosterGroup]) -> Vec<Section<'_>> {
    groups
        .iter()
        .map(|group| {
            let students = group
                .students
                .iter()
                .map(|student| (student.position, student.display_name.as_str()))
                .collect();
            (group.code.as_str(), group.size, students)
        })
        .collect()
}

#[tokio::test]
async fn the_course_groups_go_by_code_and_their_students_by_name_numbered_from_one() {
    let app = TestApp::start().await;
    let first_by_code = add_course_group(&app, "A-21").await;
    app.add_student_to_group("Zoe", &first_by_code).await;
    app.add_student_to_group("Carl", &first_by_code).await;
    app.add_student("Bob").await;
    app.add_student("Alice").await;
    app.add_outsider_student().await;

    let groups = roster(&app).await;

    assert_eq!(
        sections(&groups),
        [
            ("A-21", 2, vec![(1, "Carl"), (2, "Zoe")]),
            (
                GROUP_CODE,
                3,
                vec![
                    (1, "Alice"),
                    (2, "Bob"),
                    (3, app.fixture.student.display_name().as_str())
                ]
            ),
        ]
    );
}

#[tokio::test]
async fn a_group_nobody_has_joined_yet_is_a_section_of_size_zero() {
    let app = TestApp::start().await;
    let empty = app.add_group_to_course(&app.fixture.course).await;

    let groups = roster(&app).await;

    let section = groups
        .iter()
        .find(|group| group.code == empty.code().as_str())
        .expect("the empty group is listed");
    assert_eq!((section.size, section.students.len()), (0, 0));
}

#[tokio::test]
async fn an_assignment_of_part_of_the_groups_counts_only_for_those_groups() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    let course = app.reload_course(&app.fixture.course).await;
    for title in ["Second only", "Second again"] {
        let assignment = wall_clock_assignment(&course, &app.fixture.teacher)
            .title(title)
            .groups([&second])
            .published()
            .build();
        insert_assignment(&app.pool, &assignment).await;
    }

    let groups = roster(&app).await;

    let issued: Vec<(&str, u32)> = groups
        .iter()
        .map(|group| (group.code.as_str(), group.issued))
        .collect();
    assert_eq!(issued, [(GROUP_CODE, 1), (second.code().as_str(), 2)]);
}

#[tokio::test]
async fn an_assignment_counts_once_published_even_archived_later_while_a_draft_does_not() {
    let app = TestApp::start().await;
    app.add_draft_assignment().await;
    let archived_draft = app.add_draft_assignment().await;
    app.archive_assignment(archived_draft.id().as_uuid(), app.teacher())
        .await
        .ok();
    let archived = app
        .add_published_assignment(&app.fixture.course, &app.fixture.teacher, "Archived")
        .await;
    app.archive_assignment(archived.id().as_uuid(), app.teacher())
        .await
        .ok();

    let groups = roster(&app).await;

    assert_eq!(groups[0].issued, 2);
}

#[tokio::test]
async fn an_accepted_pass_fail_assignment_counts_in_both_numbers() {
    let app = TestApp::start().await;
    assert_eq!(app.fixture.assignment.grading(), Grading::PassFail);
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    let groups = roster(&app).await;

    assert_eq!((groups[0].issued, groups[0].students[0].accepted), (1, 1));
}

#[tokio::test]
async fn an_accepted_work_and_an_accepted_makeup_of_one_assignment_count_once() {
    let app = TestApp::start().await;
    let now = Utc::now();
    let work = assignment_of(&app.fixture.course, &app.fixture.teacher)
        .created_at(now - Duration::days(60))
        .deadline_at(now - Duration::days(40))
        .scored(10)
        .makeup_credit(6)
        .published()
        .build();
    insert_assignment(&app.pool, &work).await;
    let regular = submission_of(&work, &app.fixture.student)
        .submitted_at(now - Duration::days(41))
        .build();
    db::submissions::insert(&app.pool, &regular).await.unwrap();
    app.review_with_score(
        regular.id().as_uuid(),
        app.teacher(),
        "accept",
        out_of(5, 10),
        "Принято",
    )
    .await
    .ok();
    app.enable_makeup(3).await;
    let makeup = submission_id(
        &app.submit(work.id().as_uuid(), app.student(), "pull/makeup")
            .await
            .created(),
    );
    app.review(makeup, app.teacher(), "accept", "Досдано")
        .await
        .ok();

    let groups = roster(&app).await;

    assert_eq!(groups[0].students[0].accepted, 1);
}

#[tokio::test]
async fn a_pending_or_rejected_submission_does_not_count() {
    let app = TestApp::start().await;
    let rejected = app.queued_submission(app.student(), "pull/1").await;
    app.review(rejected, app.teacher(), "reject", "Доработайте")
        .await
        .ok();
    app.queued_submission(app.student(), "pull/2").await;

    let groups = roster(&app).await;

    assert_eq!(groups[0].students[0].accepted, 0);
}

#[tokio::test]
async fn the_next_page_repeats_the_size_of_the_group_and_continues_its_numbering() {
    let app = TestApp::start().await;
    for index in 1..=ROSTER_PAGE_SIZE {
        app.add_student(&format!("Student {index:03}")).await;
    }
    let course_id = app.fixture.course.id();

    let first = read_roster(&app, app.teacher_actor(), course_id, 0)
        .await
        .unwrap();
    let second = read_roster(&app, app.teacher_actor(), course_id, 1)
        .await
        .unwrap();

    let numbering = |roster: &CourseRoster| -> Vec<(u32, Vec<u32>)> {
        roster
            .groups
            .iter()
            .map(|group| {
                let positions = group.students.iter().map(|student| student.position);
                (group.size, positions.collect())
            })
            .collect()
    };
    let size = ROSTER_PAGE_SIZE as u32 + 1;
    assert_eq!((first.page, first.next_page), (0, Some(1)));
    assert_eq!(numbering(&first), [(size, (1..size).collect())]);
    assert_eq!((second.page, second.next_page), (1, None));
    assert_eq!(numbering(&second), [(size, vec![size])]);
}

#[tokio::test]
async fn a_teacher_outside_the_course_and_a_student_are_denied() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    for actor in [Actor::new(outsider.id()), app.student_actor()] {
        let error = read_roster(&app, actor, app.fixture.course.id(), 0)
            .await
            .unwrap_err();
        assert_eq!(error.code(), ErrorCode::PermissionDenied);
    }
}

#[tokio::test]
async fn an_admin_reads_the_roster_without_teaching_the_course() {
    let app = TestApp::start().await;
    let admin = app.add_admin().await;

    let roster = read_roster(&app, Actor::new(admin.id()), app.fixture.course.id(), 0)
        .await
        .unwrap();

    assert_eq!(
        roster.groups[0].students[0].display_name,
        app.fixture.student.display_name().as_str()
    );
}

#[tokio::test]
async fn a_sandbox_course_counts_issued_and_accepted_assignments_the_same_way() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let trial = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, "Trial")
        .await;
    let submission = submission_id(
        &app.submit(trial.id().as_uuid(), app.student(), "pull/trial")
            .await
            .created(),
    );
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();

    let roster = read_roster(&app, app.teacher_actor(), sandbox.id(), 0)
        .await
        .unwrap();

    assert_eq!(
        (
            roster.groups[0].issued,
            roster.groups[0].students[0].accepted
        ),
        (1, 1)
    );
}
