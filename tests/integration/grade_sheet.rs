use chrono::{Duration, Utc};
use hw_bot::app::{
    Actor, ChangeTerms, ErrorCode, GradeSheet, GradeSheetCell, GradeSheetGroup, GradeSheetStudent,
    LateMark, ScoreCell, WithdrawSubmission,
};
use hw_bot::db;
use hw_bot::domain::{Assignment, DeductedFrom, MakeupTerms, SubmissionId, TeachingCourseId, User};
use hw_bot::pagination::{PageRequest, ROSTER_PAGE_SIZE};
use hw_bot::test_support::{
    GROUP_CODE, assignment as assignment_of, student_group, submission as submission_of,
};
use uuid::Uuid;

use crate::harness::{TestApp, insert_assignment, out_of, submission_id, wall_clock_assignment};

async fn read_sheet(app: &TestApp, course: TeachingCourseId) -> Option<GradeSheet> {
    app.application
        .courses
        .grade_sheet(app.teacher_actor(), course)
        .await
        .expect("read the grade sheet")
}

async fn sheet(app: &TestApp) -> GradeSheet {
    read_sheet(app, app.fixture.course.id())
        .await
        .expect("the course has participants")
}

fn group<'a>(sheet: &'a GradeSheet, code: &str) -> &'a GradeSheetGroup {
    sheet
        .groups
        .iter()
        .find(|group| group.code == code)
        .unwrap_or_else(|| panic!("no sheet for {code}"))
}

fn titles(group: &GradeSheetGroup) -> Vec<&str> {
    group
        .columns
        .iter()
        .map(|column| column.title.as_str())
        .collect()
}

fn names(group: &GradeSheetGroup) -> Vec<&str> {
    group
        .students
        .iter()
        .map(|student| student.display_name.as_str())
        .collect()
}

fn student<'a>(group: &'a GradeSheetGroup, user: &User) -> &'a GradeSheetStudent {
    group
        .students
        .iter()
        .find(|student| student.student_id == user.id())
        .expect("the student is on the sheet")
}

fn cell(sheet: &GradeSheet, user: &User, assignment: &Assignment) -> GradeSheetCell {
    let group = group(sheet, GROUP_CODE);
    let position = group
        .columns
        .iter()
        .position(|column| column.assignment_id == assignment.id())
        .expect("the assignment is a column");
    student(group, user).cells[position]
}

async fn submit(app: &TestApp, assignment: &Assignment) -> Uuid {
    submission_id(
        &app.submit(assignment.id().as_uuid(), app.student(), "pull/1")
            .await
            .created(),
    )
}

async fn accepted_with_score(app: &TestApp, assignment: &Assignment, score: u16) {
    let submission = submit(app, assignment).await;
    let max_score = assignment
        .grading()
        .max_score()
        .expect("a scored assignment");
    app.review_with_score(
        submission,
        app.teacher(),
        "accept",
        out_of(score, max_score),
        "Ок",
    )
    .await
    .ok();
}

async fn accepted(app: &TestApp, submission: Uuid) {
    app.review(submission, app.teacher(), "accept", "Зачтено")
        .await
        .ok();
}

async fn rejected(app: &TestApp, submission: Uuid) {
    app.review(submission, app.teacher(), "reject", "Переделать")
        .await
        .ok();
}

async fn withdrawn(app: &TestApp, submission: Uuid) {
    app.application
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: app.student_actor(),
            withdrawn_at: Utc::now(),
        })
        .await
        .expect("the student withdraws");
}

async fn closed_with_accepted_work(app: &TestApp, score: u16) -> Assignment {
    let now = Utc::now();
    let work = assignment_of(&app.fixture.course, &app.fixture.teacher)
        .title("Closed")
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
        out_of(score, 10),
        "Принято",
    )
    .await
    .ok();

    work
}

async fn lower_makeup_credit(app: &TestApp, work: &Assignment, credit: u16) {
    let stored = db::assignments::find_versioned_by_id(&app.pool, work.id())
        .await
        .unwrap()
        .unwrap();
    app.application
        .assignments
        .change_terms(ChangeTerms {
            assignment_id: work.id(),
            actor: app.teacher_actor(),
            deadline_at: work.deadline_at(),
            late_policy: work.late_policy(),
            makeup: Some(MakeupTerms::scored(credit, work.grading()).unwrap()),
            expected_revision: stored.revision,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn sheets_go_by_group_code_and_students_by_name() {
    let app = TestApp::start().await;
    let first_by_code = student_group("A-21");
    db::academic::insert_student_group(&app.pool, &first_by_code)
        .await
        .unwrap();
    db::academic::add_teaching_course_group(&app.pool, app.fixture.course.id(), first_by_code.id())
        .await
        .unwrap();
    app.add_student_to_group("Zoe", &first_by_code).await;
    app.add_student_to_group("Carl", &first_by_code).await;
    app.add_student("Aaron").await;

    let sheet = sheet(&app).await;

    let codes: Vec<&str> = sheet
        .groups
        .iter()
        .map(|group| group.code.as_str())
        .collect();
    assert_eq!(codes, ["A-21", GROUP_CODE]);
    assert_eq!(names(&sheet.groups[0]), ["Carl", "Zoe"]);
    assert_eq!(
        names(&sheet.groups[1]),
        ["Aaron", app.fixture.student.display_name().as_str()]
    );
}

#[tokio::test]
async fn a_student_of_the_group_who_did_not_enroll_is_not_on_the_sheet() {
    let app = TestApp::start().await;
    app.add_unenrolled_student("Not enrolled").await;

    let sheet = sheet(&app).await;

    assert_eq!(
        names(group(&sheet, GROUP_CODE)),
        [app.fixture.student.display_name().as_str()]
    );
}

#[tokio::test]
async fn a_sandbox_course_lists_its_whole_group_without_enrollment() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    app.add_unenrolled_student("Not enrolled").await;

    let sheet = read_sheet(&app, sandbox.id()).await.unwrap();

    assert!(names(group(&sheet, GROUP_CODE)).contains(&"Not enrolled"));
}

#[tokio::test]
async fn an_assignment_of_another_group_is_a_column_only_on_its_own_sheet() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    app.add_student_to_group("Second", &second).await;
    let course = app.reload_course(&app.fixture.course).await;
    let only_second = wall_clock_assignment(&course, &app.fixture.teacher)
        .title("Second only")
        .groups([&second])
        .published()
        .build();
    insert_assignment(&app.pool, &only_second).await;

    let sheet = sheet(&app).await;

    assert!(!titles(group(&sheet, GROUP_CODE)).contains(&"Second only"));
    assert_eq!(
        titles(group(&sheet, second.code().as_str())),
        ["Second only"]
    );
}

#[tokio::test]
async fn a_draft_is_not_a_column_while_an_assignment_archived_after_publishing_is() {
    let app = TestApp::start().await;
    let draft = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
        .title("Draft")
        .build();
    insert_assignment(&app.pool, &draft).await;
    let archived_draft = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
        .title("Archived draft")
        .build();
    insert_assignment(&app.pool, &archived_draft).await;
    app.archive_assignment(archived_draft.id().as_uuid(), app.teacher())
        .await
        .ok();
    let archived = app
        .add_published_assignment(&app.fixture.course, &app.fixture.teacher, "Archived")
        .await;
    app.archive_assignment(archived.id().as_uuid(), app.teacher())
        .await
        .ok();

    let sheet = sheet(&app).await;

    let titles = titles(group(&sheet, GROUP_CODE));
    assert!(titles.contains(&"Archived"), "{titles:?}");
    assert!(!titles.contains(&"Draft"), "{titles:?}");
    assert!(!titles.contains(&"Archived draft"), "{titles:?}");
}

#[tokio::test]
async fn a_group_without_participants_gets_no_sheet() {
    let app = TestApp::start().await;
    let empty = app.add_group_to_course(&app.fixture.course).await;

    let sheet = sheet(&app).await;

    assert!(
        sheet
            .groups
            .iter()
            .all(|group| group.code != empty.code().as_str())
    );
}

#[tokio::test]
async fn a_participant_of_a_group_without_assignments_is_listed_without_columns() {
    let app = TestApp::start().await;
    let second = app.add_group_to_course(&app.fixture.course).await;
    app.add_student_to_group("Second", &second).await;

    let sheet = sheet(&app).await;

    let second = group(&sheet, second.code().as_str());
    assert!(second.columns.is_empty());
    assert_eq!(names(second), ["Second"]);
    assert_eq!(
        (second.students[0].accepted, second.students[0].total),
        (0, 0)
    );
}

#[tokio::test]
async fn a_course_without_participants_has_no_sheet() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await;

    let sheet = app
        .application
        .courses
        .grade_sheet(Actor::new(foreign.teacher.id()), foreign.course.id())
        .await
        .unwrap();

    assert!(sheet.is_none());
}

#[tokio::test]
async fn scored_columns_go_before_pass_fail_columns_each_by_deadline() {
    let app = TestApp::start().await;
    let now = Utc::now();
    for (title, days, scored) in [
        ("Pass late", 4, false),
        ("Score late", 3, true),
        ("Pass early", 2, false),
        ("Score early", 1, true),
    ] {
        let mut builder = wall_clock_assignment(&app.fixture.course, &app.fixture.teacher)
            .title(title)
            .deadline_at(now + Duration::days(days));
        if scored {
            builder = builder.scored(10);
        }
        insert_assignment(&app.pool, &builder.published().build()).await;
    }

    let sheet = sheet(&app).await;

    let ordered: Vec<&str> = titles(group(&sheet, GROUP_CODE))
        .into_iter()
        .filter(|title| title.contains("early") || title.contains("late"))
        .collect();
    assert_eq!(
        ordered,
        ["Score early", "Score late", "Pass early", "Pass late"]
    );
}

#[tokio::test]
async fn a_rejected_work_followed_by_a_withdrawn_one_stays_rejected() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    rejected(&app, first).await;
    let second = app.queued_submission(app.student(), "pull/2").await;
    withdrawn(&app, second).await;

    let sheet = sheet(&app).await;

    assert_eq!(
        cell(&sheet, &app.fixture.student, &app.fixture.assignment),
        GradeSheetCell::Rejected
    );
}

#[tokio::test]
async fn a_work_that_was_only_withdrawn_leaves_the_cell_empty() {
    let app = TestApp::start().await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    withdrawn(&app, submission).await;

    let sheet = sheet(&app).await;

    assert_eq!(
        cell(&sheet, &app.fixture.student, &app.fixture.assignment),
        GradeSheetCell::Empty
    );
}

#[tokio::test]
async fn a_new_work_after_a_rejection_is_pending() {
    let app = TestApp::start().await;
    let first = app.queued_submission(app.student(), "pull/1").await;
    rejected(&app, first).await;
    app.queued_submission(app.student(), "pull/2").await;

    let sheet = sheet(&app).await;

    assert_eq!(
        cell(&sheet, &app.fixture.student, &app.fixture.assignment),
        GradeSheetCell::Pending
    );
}

#[tokio::test]
async fn a_late_grade_reaches_the_cell_with_its_frozen_arithmetic() {
    let app = TestApp::start().await;
    let late = app
        .add_late_assignment("Late", 10, 2, DeductedFrom::CourseTotal)
        .await;
    accepted_with_score(&app, &late, 8).await;

    let sheet = sheet(&app).await;

    assert_eq!(
        cell(&sheet, &app.fixture.student, &late),
        GradeSheetCell::Score(ScoreCell {
            contribution: 6,
            late: Some(LateMark::Removed {
                points: 2,
                mode: None,
            }),
            makeup: false,
            frozen_scale: None,
        })
    );
}

#[tokio::test]
async fn the_better_of_a_regular_work_and_a_makeup_is_shown_like_in_the_gradebook() {
    let app = TestApp::start().await;
    let improved = closed_with_accepted_work(&app, 4).await;
    let kept = closed_with_accepted_work(&app, 5).await;
    app.enable_makeup(3).await;
    let improving = submit(&app, &improved).await;
    accepted(&app, improving).await;
    let credited_lower = submit(&app, &kept).await;
    lower_makeup_credit(&app, &kept, 4).await;
    accepted(&app, credited_lower).await;

    let sheet = sheet(&app).await;

    let contribution = |work| match cell(&sheet, &app.fixture.student, work) {
        GradeSheetCell::Score(score) => (score.contribution, score.makeup),
        other => panic!("{other:?}"),
    };
    assert_eq!(contribution(&improved), (6, true));
    assert_eq!(contribution(&kept), (5, false));
    let row = student(group(&sheet, GROUP_CODE), &app.fixture.student);
    assert_eq!(row.accepted, 2);
}

#[tokio::test]
async fn a_makeup_awaiting_review_over_an_accepted_work_keeps_the_score_and_counts_as_pending() {
    let app = TestApp::start().await;
    let work = closed_with_accepted_work(&app, 4).await;
    app.enable_makeup(3).await;
    submit(&app, &work).await;

    let sheet = sheet(&app).await;

    assert!(matches!(
        cell(&sheet, &app.fixture.student, &work),
        GradeSheetCell::Score(ScoreCell {
            contribution: 4,
            ..
        })
    ));
    let row = student(group(&sheet, GROUP_CODE), &app.fixture.student);
    assert_eq!((row.accepted, row.pending), (1, 1));
}

async fn add_every_state(app: &TestApp) {
    let on_time = app.add_scored_assignment(10).await;
    accepted_with_score(app, &on_time, 9).await;
    let to_work = app
        .add_late_assignment("To work", 10, 3, DeductedFrom::WorkScore)
        .await;
    accepted_with_score(app, &to_work, 8).await;
    let to_total = app
        .add_late_assignment("To total", 10, 4, DeductedFrom::CourseTotal)
        .await;
    accepted_with_score(app, &to_total, 2).await;
    let above_score = app
        .add_late_assignment("Above the score", 10, 5, DeductedFrom::WorkScore)
        .await;
    accepted_with_score(app, &above_score, 1).await;
    let pass = app.queued_submission(app.student(), "pull/pass").await;
    accepted(app, pass).await;

    let rescaled = app.add_scored_assignment(10).await;
    accepted_with_score(app, &rescaled, 8).await;
    app.change_grading(rescaled.id().as_uuid(), app.teacher(), Some(20))
        .await
        .ok();
    let became_pass_fail = app.add_scored_assignment(5).await;
    accepted_with_score(app, &became_pass_fail, 4).await;
    app.change_grading(became_pass_fail.id().as_uuid(), app.teacher(), None)
        .await
        .ok();

    let improved = closed_with_accepted_work(app, 4).await;
    let credited = app.add_closed_assignment("Credited", 10, 6).await;
    app.enable_makeup(5).await;
    for work in [&improved, &credited] {
        let makeup = submit(app, work).await;
        accepted(app, makeup).await;
    }

    let pending = app.add_scored_assignment(5).await;
    submit(app, &pending).await;
    let refused = app.add_scored_assignment(7).await;
    let refused = submit(app, &refused).await;
    rejected(app, refused).await;
    let taken_back = app.add_scored_assignment(3).await;
    let taken_back = submit(app, &taken_back).await;
    withdrawn(app, taken_back).await;
    app.add_scored_assignment(4).await;
}

#[tokio::test]
async fn the_sum_and_the_maximum_match_the_student_card_on_every_state() {
    let app = TestApp::start().await;
    add_every_state(&app).await;
    let idle = app.add_student("Idle").await;

    let sheet = sheet(&app).await;

    let group = group(&sheet, GROUP_CODE);
    for user in [&app.fixture.student, &idle] {
        let card = app
            .application
            .courses
            .student_grades_card(
                app.teacher_actor(),
                app.fixture.course.id(),
                user.id(),
                PageRequest::default(),
                Utc::now(),
            )
            .await
            .unwrap();
        let row = student(group, user);
        assert_eq!(row.total, card.grades.total(), "{}", row.display_name);
        assert_eq!(
            row.max_total,
            card.grades.max_total(),
            "{}",
            row.display_name
        );
        let cells_sum: i64 = row
            .cells
            .iter()
            .map(|cell| match cell {
                GradeSheetCell::Score(score) => i64::from(score.contribution),
                _ => 0,
            })
            .sum();
        assert_eq!(cells_sum, row.total, "{}", row.display_name);
    }
}

#[tokio::test]
async fn the_accepted_count_matches_the_roster_on_every_state() {
    let app = TestApp::start().await;
    add_every_state(&app).await;
    app.add_student("Idle").await;

    let sheet = sheet(&app).await;
    let roster = app
        .application
        .courses
        .roster(
            app.teacher_actor(),
            app.fixture.course.id(),
            PageRequest::sized(0, ROSTER_PAGE_SIZE),
        )
        .await
        .unwrap();

    let group = group(&sheet, GROUP_CODE);
    let listed = roster
        .groups
        .iter()
        .find(|listed| listed.code == GROUP_CODE)
        .unwrap();
    assert_eq!(listed.issued as usize, group.columns.len());
    let on_sheet: Vec<(&str, u32)> = group
        .students
        .iter()
        .map(|student| (student.display_name.as_str(), student.accepted))
        .collect();
    let on_roster: Vec<(&str, u32)> = listed
        .students
        .iter()
        .map(|student| (student.display_name.as_str(), student.accepted))
        .collect();
    assert_eq!(on_sheet, on_roster);
}

#[tokio::test]
async fn a_teacher_of_another_course_is_denied() {
    let app = TestApp::start().await;
    let outsider = app.add_outsider_teacher("Outsider").await;

    let error = app
        .application
        .courses
        .grade_sheet(Actor::new(outsider.id()), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_student_of_the_course_is_denied() {
    let app = TestApp::start().await;

    let error = app
        .application
        .courses
        .grade_sheet(app.student_actor(), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_closed_course_keeps_its_assignments_and_grades_on_the_sheet() {
    let app = TestApp::start().await;
    let scored = app.add_scored_assignment(10).await;
    accepted_with_score(&app, &scored, 7).await;
    app.mark_course_closed(&app.fixture.course).await;

    let sheet = sheet(&app).await;

    assert!(matches!(
        cell(&sheet, &app.fixture.student, &scored),
        GradeSheetCell::Score(ScoreCell {
            contribution: 7,
            ..
        })
    ));
}
