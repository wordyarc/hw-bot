use std::collections::BTreeSet;

use chrono::Utc;
use hw_bot::app::{
    Actor, ErrorCode, GrantExtraAttempts, OfferedCourse, RegisterStudent, SubmitHomework,
};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, InviteGeneration, StudentGroupId, TeachingCourse, TeachingCourseId, TelegramUserId,
    User,
};
use hw_bot::invites::VerifiedInvite;
use hw_bot::pagination::{PageRequest, ROSTER_PAGE_SIZE};
use hw_bot::test_support::{ASSIGNMENT_TITLE, GROUP_CODE, SUBJECT_NAME};

use crate::harness::TestApp;

const SECOND_SUBJECT: &str = "Android";
const SECOND_TITLE: &str = "Activity lifecycle";

fn actor(user: &User) -> Actor {
    Actor::new(user.id())
}

fn names(courses: &[OfferedCourse]) -> Vec<&str> {
    courses
        .iter()
        .map(|course| course.subject.name().as_str())
        .collect()
}

fn invite_of(app: &TestApp, course: &TeachingCourse) -> VerifiedInvite {
    app.invites
        .decode(&app.invites.encode(course.id(), InviteGeneration::FIRST))
        .unwrap()
}

async fn second_course(app: &TestApp) -> (TeachingCourse, Assignment) {
    let course = app.add_subject_course(SECOND_SUBJECT).await;
    let assignment = app
        .add_published_assignment(&course, &app.fixture.teacher, SECOND_TITLE)
        .await;

    (course, assignment)
}

async fn enroll(app: &TestApp, student: &User, courses: &[&TeachingCourse]) {
    let ids: Vec<_> = courses.iter().map(|course| course.id()).collect();
    db::enrollments::insert(&app.pool, student.id(), &ids)
        .await
        .unwrap();
}

async fn visible_titles(app: &TestApp, student: &User) -> Vec<String> {
    app.application
        .assignments
        .list_for_student(actor(student), PageRequest::default())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.assignment.title().as_str().to_owned())
        .collect()
}

async fn subjects(app: &TestApp, student: &User) -> Vec<String> {
    app.application
        .users
        .subjects(actor(student))
        .await
        .unwrap()
        .into_iter()
        .map(|subject| subject.name().as_str().to_owned())
        .collect()
}

async fn enrollments(app: &TestApp) -> i64 {
    sqlx::query_scalar("select count(*) from teaching_course_students")
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

async fn register(
    app: &TestApp,
    invite: VerifiedInvite,
    courses: Option<BTreeSet<TeachingCourseId>>,
) -> Result<User, ErrorCode> {
    app.application
        .users
        .register_student(RegisterStudent {
            telegram_user_id: TelegramUserId::new(app.next_telegram_id()).unwrap(),
            display_name: "Ivan".to_owned(),
            invite,
            student_group_id: Some(app.fixture.group.id()),
            courses,
            registered_at: Utc::now(),
        })
        .await
        .map(|registered| registered.user)
        .map_err(|error| error.code())
}

#[tokio::test]
async fn enrolling_twice_keeps_a_single_enrollment() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Late").await;

    enroll(&app, &student, &[&app.fixture.course]).await;
    enroll(&app, &student, &[&app.fixture.course]).await;

    let stored: i64 =
        sqlx::query_scalar("select count(*) from teaching_course_students where student_id = $1")
            .bind(student.id().as_uuid())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(stored, 1);
}

#[tokio::test]
async fn a_group_is_offered_its_courses_by_subject_without_the_sandbox_and_foreign_courses() {
    let app = TestApp::start().await;
    app.add_subject_course(SECOND_SUBJECT).await;
    app.add_sandbox_course().await;
    app.add_foreign_course().await;

    let offered = db::enrollments::offered_courses(&app.pool, app.fixture.group.id(), None)
        .await
        .unwrap();

    assert_eq!(names(&offered), [SECOND_SUBJECT, SUBJECT_NAME]);
}

#[tokio::test]
async fn a_student_is_offered_only_the_courses_they_have_not_enrolled_in() {
    let app = TestApp::start().await;
    app.add_subject_course(SECOND_SUBJECT).await;
    let student = app.add_unenrolled_student("Late").await;
    enroll(&app, &student, &[&app.fixture.course]).await;

    let offered = app
        .application
        .users
        .open_courses(actor(&student))
        .await
        .unwrap();

    assert_eq!(names(&offered), [SECOND_SUBJECT]);
}

#[tokio::test]
async fn a_student_outside_the_course_cannot_open_its_assignment() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Outside").await;

    let card = app
        .application
        .assignments
        .find_for_student(actor(&student), app.fixture.assignment.id())
        .await;

    assert_eq!(card.unwrap_err().code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_submission_of_a_student_outside_the_course_is_refused_and_not_stored() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Outside").await;

    let submitted = app
        .application
        .submissions
        .submit_homework(SubmitHomework {
            assignment_id: app.fixture.assignment.id(),
            actor: actor(&student),
            content: "https://example.com/pr/1".to_owned(),
            submitted_at: Utc::now(),
            allow_makeup: true,
        })
        .await;

    assert_eq!(submitted.unwrap_err().code(), ErrorCode::PermissionDenied);
    let stored: i64 = sqlx::query_scalar("select count(*) from submissions")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn extra_attempts_are_not_granted_to_a_student_outside_the_course() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Outside").await;

    let granted = app
        .application
        .assignments
        .grant_extra_attempts(GrantExtraAttempts {
            assignment_id: app.fixture.assignment.id(),
            student_id: student.id(),
            actor: app.teacher_actor(),
            amount: 1,
        })
        .await;

    assert_eq!(granted.unwrap_err().code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn a_student_outside_the_course_has_no_contact_there_even_for_an_admin() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Outside").await;
    let admin = app.add_admin().await;

    for reader in [app.teacher_actor(), actor(&admin)] {
        let contact = app
            .application
            .courses
            .student_contact(reader, app.fixture.course.id(), student.id())
            .await;

        assert_eq!(contact.unwrap_err().code(), ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn a_student_outside_the_course_has_no_grades_or_makeup_attempts_there() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Outside").await;
    let courses = &app.application.courses;
    let (teacher, course) = (app.teacher_actor(), app.fixture.course.id());

    let grades = courses
        .student_grades(teacher, course, student.id(), PageRequest::default())
        .await;
    let makeup = courses
        .student_makeup(teacher, course, student.id(), Utc::now())
        .await;
    let granted = courses
        .grant_makeup_attempt(teacher, course, student.id(), Utc::now())
        .await;

    assert_eq!(grades.unwrap_err().code(), ErrorCode::NotFound);
    assert_eq!(makeup.unwrap_err().code(), ErrorCode::NotFound);
    assert_eq!(granted.unwrap_err().code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn a_sandbox_assignment_is_open_to_a_student_without_any_enrollment() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let assignment = app
        .add_published_assignment(&sandbox, &app.fixture.teacher, SECOND_TITLE)
        .await;
    let student = app.add_unenrolled_student("Newcomer").await;

    let card = app
        .application
        .assignments
        .find_for_student(actor(&student), assignment.id())
        .await;

    assert!(card.is_ok(), "{:?}", card.err().map(|error| error.code()));
    assert_eq!(visible_titles(&app, &student).await, [SECOND_TITLE]);
    assert!(subjects(&app, &student).await.is_empty());
}

#[tokio::test]
async fn a_student_lists_only_the_assignments_of_the_courses_they_are_enrolled_in() {
    let app = TestApp::start().await;
    second_course(&app).await;
    let student = app.add_unenrolled_student("Chooser").await;
    enroll(&app, &student, &[&app.fixture.course]).await;

    assert_eq!(visible_titles(&app, &student).await, [ASSIGNMENT_TITLE]);
}

#[tokio::test]
async fn the_archive_keeps_only_the_assignments_of_the_courses_a_student_is_enrolled_in() {
    let app = TestApp::start().await;
    let (_, foreign) = second_course(&app).await;
    let student = app.add_unenrolled_student("Chooser").await;
    enroll(&app, &student, &[&app.fixture.course]).await;
    for assignment in [app.assignment(), foreign.id().as_uuid()] {
        app.archive_assignment(assignment, app.teacher()).await.ok();
    }

    let archived: Vec<_> = app
        .application
        .assignments
        .list_archived_for_student(actor(&student), PageRequest::default())
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.assignment.id())
        .collect();

    assert_eq!(archived, [app.fixture.assignment.id()]);
}

#[tokio::test]
async fn the_profile_counts_only_the_assignments_of_the_courses_a_student_is_enrolled_in() {
    let app = TestApp::start().await;
    second_course(&app).await;
    let student = app.add_unenrolled_student("Chooser").await;
    enroll(&app, &student, &[&app.fixture.course]).await;

    let statistics = app
        .application
        .users
        .statistics(actor(&student))
        .await
        .unwrap();

    assert_eq!(statistics.assignments, 1);
}

#[tokio::test]
async fn enrolling_later_reveals_the_assignments_published_before() {
    let app = TestApp::start().await;
    let student = app.add_unenrolled_student("Late").await;
    assert!(visible_titles(&app, &student).await.is_empty());

    app.application
        .users
        .enroll(actor(&student), &BTreeSet::from([app.fixture.course.id()]))
        .await
        .unwrap();

    assert_eq!(visible_titles(&app, &student).await, [ASSIGNMENT_TITLE]);
}

#[tokio::test]
async fn the_roster_lists_and_counts_only_the_enrolled_students_of_a_group() {
    let app = TestApp::start().await;
    let empty = app.add_group_to_course(&app.fixture.course).await;
    app.add_unenrolled_student("Outside").await;
    sqlx::query("insert into users (telegram_user_id, role, display_name, student_group_id) values (77001, 'student', 'Unenrolled', $1)")
        .bind(empty.id().as_uuid())
        .execute(&app.pool)
        .await
        .unwrap();

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

    let sections: Vec<_> = roster
        .groups
        .iter()
        .map(|group| {
            let students: Vec<_> = group
                .students
                .iter()
                .map(|student| student.display_name.as_str())
                .collect();
            (group.code.as_str(), group.size, students)
        })
        .collect();
    assert_eq!(
        sections,
        [
            (GROUP_CODE, 1, vec!["Student"]),
            (empty.code().as_str(), 0, vec![]),
        ]
    );
}

#[tokio::test]
async fn the_gradebook_has_a_zero_row_for_an_enrolled_student_and_none_for_the_rest() {
    let app = TestApp::start().await;
    app.add_unenrolled_student("Outside").await;
    app.add_scored_assignment(10).await;

    let rows = app
        .application
        .courses
        .gradebook(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.group.id(),
            PageRequest::default(),
        )
        .await
        .unwrap()
        .items;

    let totals: Vec<_> = rows
        .iter()
        .map(|row| (row.display_name.as_str(), row.work_total(), row.max_total()))
        .collect();
    assert_eq!(totals, [("Student", 0, 10)]);
}

#[tokio::test]
async fn the_students_of_a_gradebook_group_are_the_enrolled_ones() {
    let app = TestApp::start().await;
    app.add_unenrolled_student("Outside").await;

    let students = app
        .application
        .courses
        .gradebook_students(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.group.id(),
            PageRequest::default(),
        )
        .await
        .unwrap()
        .items;

    let listed: Vec<_> = students
        .iter()
        .map(|student| student.display_name.as_str())
        .collect();
    assert_eq!(listed, ["Student"]);
}

#[tokio::test]
async fn the_students_of_an_assignment_are_the_enrolled_ones() {
    let app = TestApp::start().await;
    app.add_unenrolled_student("Outside").await;

    let students = app
        .application
        .assignments
        .list_students_for_assignment(
            app.teacher_actor(),
            app.fixture.assignment.id(),
            PageRequest::default(),
        )
        .await
        .unwrap()
        .students
        .items;

    let listed: Vec<_> = students
        .iter()
        .map(|summary| summary.student.display_name().as_str())
        .collect();
    assert_eq!(listed, ["Student"]);
}

#[tokio::test]
async fn a_registration_enrolls_in_exactly_the_chosen_courses() {
    let app = TestApp::start().await;
    let (second, _) = second_course(&app).await;
    app.add_subject_course("Frontend").await;
    let chosen = BTreeSet::from([app.fixture.course.id(), second.id()]);

    let student = register(&app, invite_of(&app, &app.fixture.course), Some(chosen))
        .await
        .unwrap();

    assert_eq!(
        subjects(&app, &student).await,
        [SECOND_SUBJECT, SUBJECT_NAME]
    );
    assert_eq!(
        visible_titles(&app, &student).await,
        [SECOND_TITLE, ASSIGNMENT_TITLE]
    );
}

#[tokio::test]
async fn a_registration_may_leave_the_course_of_its_link_unchosen() {
    let app = TestApp::start().await;
    let (second, _) = second_course(&app).await;

    let student = register(
        &app,
        invite_of(&app, &app.fixture.course),
        Some(BTreeSet::from([second.id()])),
    )
    .await
    .unwrap();

    assert_eq!(subjects(&app, &student).await, [SECOND_SUBJECT]);
}

#[tokio::test]
async fn a_registration_without_chosen_courses_enrolls_in_the_course_of_its_link() {
    let app = TestApp::start().await;
    second_course(&app).await;

    let student = register(&app, invite_of(&app, &app.fixture.course), None)
        .await
        .unwrap();

    assert_eq!(subjects(&app, &student).await, [SUBJECT_NAME]);
}

#[tokio::test]
async fn a_registration_by_a_sandbox_link_without_chosen_courses_enrolls_nowhere() {
    let app = TestApp::start().await;
    let sandbox = app.add_sandbox_course().await;
    let before = enrollments(&app).await;

    let student = register(&app, invite_of(&app, &sandbox), None)
        .await
        .unwrap();

    assert!(subjects(&app, &student).await.is_empty());
    assert_eq!(enrollments(&app).await, before);
}

#[tokio::test]
async fn a_foreign_or_sandbox_course_among_the_chosen_refuses_the_whole_registration() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await.course;
    let sandbox = app.add_sandbox_course().await;
    let users_before: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let enrollments_before = enrollments(&app).await;

    for unoffered in [foreign.id(), sandbox.id()] {
        let chosen = BTreeSet::from([app.fixture.course.id(), unoffered]);

        let refused = register(&app, invite_of(&app, &app.fixture.course), Some(chosen)).await;

        assert_eq!(refused.unwrap_err(), ErrorCode::InvalidInput);
    }
    let users_after: i64 = sqlx::query_scalar("select count(*) from users")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(users_after, users_before);
    assert_eq!(enrollments(&app).await, enrollments_before);
}

#[tokio::test]
async fn a_rotated_link_registers_nobody_and_enrolls_nowhere() {
    let app = TestApp::start().await;
    let invite = invite_of(&app, &app.fixture.course);
    app.application
        .courses
        .rotate_course_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap();
    let before = enrollments(&app).await;

    let refused = register(&app, invite, None).await;

    assert_eq!(refused.unwrap_err(), ErrorCode::InviteInvalid);
    assert_eq!(enrollments(&app).await, before);
}

#[tokio::test]
async fn the_courses_to_choose_are_not_listed_for_a_group_outside_the_invited_course() {
    let app = TestApp::start().await;
    let foreign: StudentGroupId = app.add_foreign_course().await.group.id();

    let offer = app
        .application
        .users
        .registration_offer(invite_of(&app, &app.fixture.course), foreign)
        .await;

    assert_eq!(offer.unwrap_err().code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn the_courses_to_choose_are_not_listed_by_a_rotated_link() {
    let app = TestApp::start().await;
    let invite = invite_of(&app, &app.fixture.course);
    app.application
        .courses
        .rotate_course_invite(
            app.teacher_actor(),
            app.fixture.course.id(),
            InviteGeneration::FIRST,
        )
        .await
        .unwrap();

    let offer = app
        .application
        .users
        .registration_offer(invite, app.fixture.group.id())
        .await;

    assert_eq!(offer.unwrap_err().code(), ErrorCode::InviteInvalid);
}

#[tokio::test]
async fn a_link_of_a_course_a_student_has_not_joined_offers_the_courses_still_open_to_them() {
    let app = TestApp::start().await;
    let (second, _) = second_course(&app).await;
    app.add_subject_course("Frontend").await;
    let student = app.add_unenrolled_student("Returning").await;
    enroll(&app, &student, &[&app.fixture.course]).await;

    let offer = app
        .application
        .users
        .enrollment_offer(actor(&student), invite_of(&app, &second))
        .await
        .unwrap()
        .expect("the course of the link is still open to the student");

    assert_eq!(names(&offer), [SECOND_SUBJECT, "Frontend"]);
}

#[tokio::test]
async fn a_link_offers_nothing_once_its_course_is_joined_foreign_a_sandbox_or_rotated() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await.course;
    let sandbox = app.add_sandbox_course().await;
    let (rotated, _) = second_course(&app).await;
    let rotated_invite = invite_of(&app, &rotated);
    let student = app.add_unenrolled_student("Returning").await;
    enroll(&app, &student, &[&app.fixture.course]).await;
    app.application
        .courses
        .rotate_course_invite(app.teacher_actor(), rotated.id(), InviteGeneration::FIRST)
        .await
        .unwrap();

    for (invite, case) in [
        (invite_of(&app, &app.fixture.course), "joined"),
        (invite_of(&app, &foreign), "foreign"),
        (invite_of(&app, &sandbox), "sandbox"),
        (rotated_invite, "rotated"),
    ] {
        let offer = app
            .application
            .users
            .enrollment_offer(actor(&student), invite)
            .await
            .unwrap();

        assert_eq!(offer, None, "{case}");
    }
}

#[tokio::test]
async fn a_link_offers_no_enrollment_to_a_teacher() {
    let app = TestApp::start().await;

    let offer = app
        .application
        .users
        .enrollment_offer(app.teacher_actor(), invite_of(&app, &app.fixture.course))
        .await
        .unwrap();

    assert_eq!(offer, None);
}

#[tokio::test]
async fn enrolling_again_succeeds_and_changes_nothing() {
    let app = TestApp::start().await;
    let chosen = BTreeSet::from([app.fixture.course.id()]);
    let before = enrollments(&app).await;

    let enrolled = app
        .application
        .users
        .enroll(app.student_actor(), &chosen)
        .await
        .unwrap();

    assert_eq!(names(&enrolled), [SUBJECT_NAME]);
    assert_eq!(enrollments(&app).await, before);
}

#[tokio::test]
async fn a_foreign_course_among_the_chosen_enrolls_in_none_of_them() {
    let app = TestApp::start().await;
    let foreign = app.add_foreign_course().await.course;
    let student = app.add_unenrolled_student("Late").await;
    let chosen = BTreeSet::from([app.fixture.course.id(), foreign.id()]);

    let refused = app.application.users.enroll(actor(&student), &chosen).await;

    assert_eq!(refused.unwrap_err().code(), ErrorCode::InvalidInput);
    assert!(subjects(&app, &student).await.is_empty());
}

#[tokio::test]
async fn staff_cannot_enroll() {
    let app = TestApp::start().await;
    let chosen = BTreeSet::from([app.fixture.course.id()]);

    let refused = app
        .application
        .users
        .enroll(app.teacher_actor(), &chosen)
        .await;

    assert_eq!(refused.unwrap_err().code(), ErrorCode::PermissionDenied);
}
