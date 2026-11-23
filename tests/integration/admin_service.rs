use std::collections::BTreeSet;
use std::time::Duration;

use axum::http::StatusCode;
use chrono::Utc;
use hw_bot::app::{
    Actor, AdminService, ChangeStudentGroup, CreateCourse, CreateStudentGroup, ErrorCode,
    RegisterTeacher, WithdrawSubmission,
};
use hw_bot::db;
use hw_bot::domain::{
    AssignmentStatus, EducationLevel, StudentGroup, StudentGroupId, Subject, SubjectId,
    SubmissionId, SubmissionStatus, TeachingCourse, TeachingCourseId, TelegramUserId, User, UserId,
};
use hw_bot::pagination::PageRequest;
use hw_bot::test_support::logs::LogBuffer;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::harness::{TestApp, out_of, wait_for_blocked_connection};

fn service(app: &TestApp) -> &AdminService {
    &app.application.admin
}

async fn admin(app: &TestApp) -> Actor {
    Actor::new(app.add_admin().await.id())
}

async fn blocking_transaction(app: &TestApp) -> (Transaction<'static, Postgres>, i32) {
    let mut transaction = app.pool.begin().await.expect("begin transaction");
    let backend = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await
        .expect("read backend pid");

    (transaction, backend)
}

async fn add_group(app: &TestApp, actor: Actor, code: &str) -> StudentGroup {
    service(app)
        .create_group(CreateStudentGroup {
            actor,
            code: code.to_owned(),
            education_level: EducationLevel::Bachelor,
            study_year: 2,
        })
        .await
        .expect("create group")
}

async fn add_subject(app: &TestApp, actor: Actor, name: &str) -> Subject {
    service(app)
        .create_subject(actor, name.to_owned())
        .await
        .expect("create subject")
}

fn new_course(
    app: &TestApp,
    actor: Actor,
    subject_id: SubjectId,
    groups: &[&StudentGroup],
) -> CreateCourse {
    CreateCourse {
        actor,
        subject_id,
        group_ids: groups.iter().map(|group| group.id()).collect(),
        teacher_ids: BTreeSet::from([app.fixture.teacher.id()]),
        created_at: Utc::now(),
    }
}

async fn add_course(
    app: &TestApp,
    actor: Actor,
    name: &str,
    groups: &[&StudentGroup],
) -> TeachingCourse {
    let subject = add_subject(app, actor, name).await;

    service(app)
        .create_course(new_course(app, actor, subject.id(), groups))
        .await
        .expect("create course")
}

async fn enrollments(app: &TestApp, student: UserId) -> BTreeSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "select teaching_course_id from teaching_course_students where student_id = $1",
    )
    .bind(student.as_uuid())
    .fetch_all(&app.pool)
    .await
    .expect("read enrollments")
    .into_iter()
    .collect()
}

async fn stored_user(app: &TestApp, id: UserId) -> User {
    db::users::find_by_id(&app.pool, id)
        .await
        .expect("read user")
        .expect("user exists")
}

fn change_group(
    actor: Actor,
    student: &User,
    group: &StudentGroup,
    courses: &[&TeachingCourse],
) -> ChangeStudentGroup {
    ChangeStudentGroup {
        actor,
        student_id: student.id(),
        group_id: group.id(),
        courses: courses.iter().map(|course| course.id()).collect(),
    }
}

async fn assignment_status(app: &TestApp, id: Uuid) -> AssignmentStatus {
    db::assignments::find_by_id(&app.pool, hw_bot::domain::AssignmentId::from_uuid(id))
        .await
        .expect("read assignment")
        .expect("assignment exists")
        .status()
}

async fn close(app: &TestApp, actor: Actor, course: TeachingCourseId) {
    service(app)
        .close_course(actor, course, Utc::now())
        .await
        .expect("close course");
}

#[tokio::test]
async fn every_operation_refuses_a_teacher_and_a_student() {
    let app = TestApp::start().await;
    let group = app.fixture.group.id();
    let subject = app.fixture.subject.id();
    let course = app.fixture.course.id();
    let student = app.fixture.student.id();
    let teacher = app.fixture.teacher.id();
    let page = PageRequest::default();
    let admin = service(&app);

    for actor in [app.teacher_actor(), app.student_actor()] {
        let codes = [
            admin.ensure_admin(actor).await.map(drop),
            admin
                .search_students(actor, "Student", page)
                .await
                .map(drop),
            admin.student_card(actor, student).await.map(drop),
            admin
                .rename_student(actor, student, "Новое имя".to_owned())
                .await
                .map(drop),
            admin
                .group_change_offer(actor, student, group)
                .await
                .map(drop),
            admin
                .change_student_group(ChangeStudentGroup {
                    actor,
                    student_id: student,
                    group_id: group,
                    courses: BTreeSet::new(),
                })
                .await,
            admin.unenroll_student(actor, student, course).await,
            admin.list_staff(actor, page).await.map(drop),
            admin.teacher_card(actor, teacher, page).await.map(drop),
            admin
                .register_teacher(RegisterTeacher {
                    actor,
                    telegram_user_id: TelegramUserId::new(777).unwrap(),
                    display_name: "Новый".to_owned(),
                    registered_at: Utc::now(),
                })
                .await
                .map(drop),
            admin.assign_teacher(actor, teacher, course).await,
            admin.unassign_teacher(actor, teacher, course).await,
            admin.list_groups(actor, page).await.map(drop),
            admin.find_group(actor, group).await.map(drop),
            admin
                .create_group(CreateStudentGroup {
                    actor,
                    code: "NEW".to_owned(),
                    education_level: EducationLevel::Master,
                    study_year: 1,
                })
                .await
                .map(drop),
            admin
                .rename_group(actor, group, "RENAMED".to_owned())
                .await
                .map(drop),
            admin.change_group_year(actor, group, 2).await.map(drop),
            admin.list_subjects(actor, page).await.map(drop),
            admin.find_subject(actor, subject).await.map(drop),
            admin
                .create_subject(actor, "Новый".to_owned())
                .await
                .map(drop),
            admin
                .rename_subject(actor, subject, "Другой".to_owned())
                .await
                .map(drop),
            admin.list_courses(actor, false, page).await.map(drop),
            admin.course_card(actor, course).await.map(drop),
            admin
                .groups_free_for_subject(actor, subject, page)
                .await
                .map(drop),
            admin
                .create_course(CreateCourse {
                    actor,
                    subject_id: subject,
                    group_ids: BTreeSet::new(),
                    teacher_ids: BTreeSet::new(),
                    created_at: Utc::now(),
                })
                .await
                .map(drop),
            admin.add_course_group(actor, course, group).await,
            admin.close_preview(actor, course).await.map(drop),
            admin
                .close_course(actor, course, Utc::now())
                .await
                .map(drop),
            admin.reopen_course(actor, course).await,
        ];

        for (index, result) in codes.into_iter().enumerate() {
            assert_eq!(
                result.unwrap_err().code(),
                ErrorCode::PermissionDenied,
                "operation #{index}"
            );
        }
    }
    let stored = stored_user(&app, student).await;
    assert_eq!(stored.display_name().as_str(), "Student");
}

#[tokio::test]
async fn a_search_treats_yo_and_ye_as_one_letter_and_shows_the_group() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let student = app.add_student("Семёнов Иван").await;

    let found = service(&app)
        .search_students(actor, "семенов", PageRequest::default())
        .await
        .unwrap();

    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].student_id, student.id());
    assert_eq!(found.items[0].group_code, app.fixture.group.code().as_str());
}

#[tokio::test]
async fn a_search_finds_a_name_written_with_ye_by_a_query_with_yo() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let student = app.add_student("Семенов Иван").await;

    let found = service(&app)
        .search_students(actor, "СЕМЁН", PageRequest::default())
        .await
        .unwrap();

    assert_eq!(found.items[0].student_id, student.id());
}

#[tokio::test]
async fn pattern_characters_in_a_search_are_literal() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    app.add_student("Обычное имя").await;
    let literal = app.add_student("Имя %% со знаком").await;

    let found = service(&app)
        .search_students(actor, "%%", PageRequest::default())
        .await
        .unwrap();

    let ids: Vec<_> = found.items.iter().map(|item| item.student_id).collect();
    assert_eq!(ids, [literal.id()]);
}

#[tokio::test]
async fn a_search_is_ordered_by_name_and_paged() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    for index in (0..12).rev() {
        app.add_student(&format!("Искомый {index:02}")).await;
    }

    let first = service(&app)
        .search_students(actor, "искомый", PageRequest::new(0))
        .await
        .unwrap();
    let second = service(&app)
        .search_students(actor, "искомый", PageRequest::new(1))
        .await
        .unwrap();

    assert_eq!(first.items[0].display_name, "Искомый 00");
    assert_eq!(first.items.len(), 10);
    assert_eq!(first.next_page, Some(1));
    assert_eq!(second.items[1].display_name, "Искомый 11");
    assert_eq!(second.next_page, None);
}

#[tokio::test]
async fn staff_stays_out_of_the_student_search() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    app.add_outsider_teacher("Искомый преподаватель").await;
    let student = app.add_student("Искомый студент").await;

    let found = service(&app)
        .search_students(actor, "искомый", PageRequest::default())
        .await
        .unwrap();

    let ids: Vec<_> = found.items.iter().map(|item| item.student_id).collect();
    assert_eq!(ids, [student.id()]);
}

#[tokio::test]
async fn a_one_letter_search_is_invalid_input() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .search_students(actor, " я ", PageRequest::default())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_student_card_counts_blocking_submissions_per_course() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let other = add_course(&app, actor, "Алгоритмы", &[&app.fixture.group]).await;
    let student = app.fixture.student.id();
    service(&app)
        .change_student_group(ChangeStudentGroup {
            actor,
            student_id: student,
            group_id: app.fixture.group.id(),
            courses: BTreeSet::from([app.fixture.course.id(), other.id()]),
        })
        .await
        .unwrap();
    app.queued_submission(app.student(), "pull/1").await;

    let card = service(&app).student_card(actor, student).await.unwrap();

    assert_eq!(card.blocking_submissions, 1);
    assert_eq!(card.group.id(), app.fixture.group.id());
    let blocking = |id: TeachingCourseId| {
        card.enrollments
            .iter()
            .find(|enrollment| enrollment.course_id == id)
            .unwrap()
            .blocking_submissions
    };
    assert_eq!(blocking(app.fixture.course.id()), 1);
    assert_eq!(blocking(other.id()), 0);
}

#[tokio::test]
async fn a_card_of_a_teacher_is_not_a_student_card() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .student_card(actor, app.fixture.teacher.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NotFound);
}

#[tokio::test]
async fn a_new_name_is_stored_on_one_line() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let student = app.fixture.student.id();

    service(&app)
        .rename_student(actor, student, "  Иванов \n  Иван\t".to_owned())
        .await
        .unwrap();

    assert_eq!(
        stored_user(&app, student).await.display_name().as_str(),
        "Иванов Иван"
    );
}

#[tokio::test]
async fn a_blank_name_is_rejected_and_the_old_one_stays() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let student = app.fixture.student.id();

    let error = service(&app)
        .rename_student(actor, student, " \n ".to_owned())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert_eq!(
        stored_user(&app, student).await.display_name().as_str(),
        "Student"
    );
}

#[tokio::test]
async fn renaming_keeps_submissions() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    service(&app)
        .rename_student(actor, app.fixture.student.id(), "Новое имя".to_owned())
        .await
        .unwrap();

    let stored = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status(), SubmissionStatus::PendingReview);
    assert_eq!(stored.student_id(), app.fixture.student.id());
}

#[tokio::test]
async fn a_group_change_stores_exactly_the_chosen_courses() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;
    let first = add_course(&app, actor, "Алгоритмы", &[&group]).await;
    let second = add_course(&app, actor, "Базы данных", &[&group]).await;
    add_course(&app, actor, "Не выбран", &[&group]).await;
    let student = &app.fixture.student;

    service(&app)
        .change_student_group(change_group(actor, student, &group, &[&first, &second]))
        .await
        .unwrap();

    assert_eq!(
        stored_user(&app, student.id()).await.student_group_id(),
        Some(group.id())
    );
    assert_eq!(
        enrollments(&app, student.id()).await,
        BTreeSet::from([first.id().as_uuid(), second.id().as_uuid()])
    );
}

#[tokio::test]
async fn an_enrollment_of_the_old_group_does_not_revive() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;
    let student = &app.fixture.student;
    service(&app)
        .change_student_group(change_group(actor, student, &group, &[]))
        .await
        .unwrap();

    service(&app)
        .add_course_group(actor, app.fixture.course.id(), group.id())
        .await
        .unwrap();

    let participation =
        db::enrollments::participation(&app.pool, app.fixture.course.id(), student.id())
            .await
            .unwrap();
    assert_eq!(participation, hw_bot::domain::Participation::Outsider);
}

#[tokio::test]
async fn the_group_change_offer_marks_current_enrollments() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;
    service(&app)
        .add_course_group(actor, app.fixture.course.id(), group.id())
        .await
        .unwrap();
    let other = add_course(&app, actor, "Алгоритмы", &[&group]).await;

    let offer = service(&app)
        .group_change_offer(actor, app.fixture.student.id(), group.id())
        .await
        .unwrap();

    assert_eq!(offer.courses.len(), 2);
    assert!(offer.enrolled.contains(&app.fixture.course.id()));
    assert!(!offer.enrolled.contains(&other.id()));
}

#[tokio::test]
async fn a_withdrawn_submission_does_not_block_a_group_change() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.application
        .submissions
        .withdraw(WithdrawSubmission {
            submission_id: SubmissionId::from_uuid(submission),
            actor: app.student_actor(),
            withdrawn_at: Utc::now(),
        })
        .await
        .unwrap();

    service(&app)
        .change_student_group(change_group(actor, &app.fixture.student, &group, &[]))
        .await
        .unwrap();

    let stored = db::submissions::find_by_id(&app.pool, SubmissionId::from_uuid(submission))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status(), SubmissionStatus::Withdrawn);
}

#[tokio::test]
async fn every_other_submission_status_blocks_a_group_change() {
    for decision in [None, Some("accept"), Some("reject")] {
        let app = TestApp::start().await;
        let actor = admin(&app).await;
        let group = add_group(&app, actor, "NEW-GROUP").await;
        let submission = app.queued_submission(app.student(), "pull/1").await;
        if let Some(decision) = decision {
            app.review(submission, app.teacher(), decision, "Итог")
                .await
                .ok();
        }
        let before = enrollments(&app, app.fixture.student.id()).await;

        let error = service(&app)
            .change_student_group(change_group(actor, &app.fixture.student, &group, &[]))
            .await
            .unwrap_err();

        assert_eq!(
            error.code(),
            ErrorCode::StudentHasSubmissions,
            "{decision:?}"
        );
        assert_eq!(
            stored_user(&app, app.fixture.student.id())
                .await
                .student_group_id(),
            Some(app.fixture.group.id())
        );
        assert_eq!(enrollments(&app, app.fixture.student.id()).await, before);
    }
}

#[tokio::test]
async fn a_course_outside_the_new_group_is_refused() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;

    let error = service(&app)
        .change_student_group(change_group(
            actor,
            &app.fixture.student,
            &group,
            &[&app.fixture.course],
        ))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn removing_from_a_subject_keeps_other_enrollments() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let other = add_course(&app, actor, "Алгоритмы", &[&app.fixture.group]).await;
    let student = app.add_unenrolled_student("Двойной").await;
    db::enrollments::insert(
        &app.pool,
        student.id(),
        &[app.fixture.course.id(), other.id()],
    )
    .await
    .unwrap();

    service(&app)
        .unenroll_student(actor, student.id(), app.fixture.course.id())
        .await
        .unwrap();

    assert_eq!(
        enrollments(&app, student.id()).await,
        BTreeSet::from([other.id().as_uuid()])
    );
}

#[tokio::test]
async fn a_submission_in_another_course_does_not_block_removal() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let other = add_course(&app, actor, "Алгоритмы", &[&app.fixture.group]).await;
    db::enrollments::insert(&app.pool, app.fixture.student.id(), &[other.id()])
        .await
        .unwrap();
    app.queued_submission(app.student(), "pull/1").await;

    service(&app)
        .unenroll_student(actor, app.fixture.student.id(), other.id())
        .await
        .unwrap();

    assert!(
        !enrollments(&app, app.fixture.student.id())
            .await
            .contains(&other.id().as_uuid())
    );
}

#[tokio::test]
async fn a_submission_in_the_course_blocks_removal() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    app.queued_submission(app.student(), "pull/1").await;

    let error = service(&app)
        .unenroll_student(actor, app.fixture.student.id(), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::StudentHasSubmissions);
    assert!(
        enrollments(&app, app.fixture.student.id())
            .await
            .contains(&app.fixture.course.id().as_uuid())
    );
}

#[tokio::test]
async fn a_removed_student_can_enroll_again() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let course = app.fixture.course.id();
    service(&app)
        .unenroll_student(actor, app.fixture.student.id(), course)
        .await
        .unwrap();

    app.application
        .users
        .enroll(app.student_actor(), &BTreeSet::from([course]))
        .await
        .unwrap();

    assert!(
        enrollments(&app, app.fixture.student.id())
            .await
            .contains(&course.as_uuid())
    );
}

fn register(actor: Actor, telegram_id: i64) -> RegisterTeacher {
    RegisterTeacher {
        actor,
        telegram_user_id: TelegramUserId::new(telegram_id).unwrap(),
        display_name: "Новый преподаватель".to_owned(),
        registered_at: Utc::now(),
    }
}

#[tokio::test]
async fn a_registered_teacher_has_the_teacher_role() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let teacher = service(&app)
        .register_teacher(register(actor, 91_000))
        .await
        .unwrap();

    assert!(matches!(
        stored_user(&app, teacher.id()).await.role(),
        hw_bot::domain::Role::Teacher
    ));
}

#[tokio::test]
async fn a_taken_telegram_id_is_already_registered_and_unchanged() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .register_teacher(register(actor, app.student()))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::AlreadyRegistered);
    assert!(
        stored_user(&app, app.fixture.student.id())
            .await
            .role()
            .can_submit_homework()
    );
}

#[tokio::test]
async fn assigning_a_teacher_twice_is_idempotent() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let teacher = app.add_outsider_teacher("Второй").await;
    let course = app.fixture.course.id();

    for _ in 0..2 {
        service(&app)
            .assign_teacher(actor, teacher.id(), course)
            .await
            .unwrap();
    }

    let card = service(&app).course_card(actor, course).await.unwrap();
    assert_eq!(card.teachers.len(), 2);
}

#[tokio::test]
async fn a_teacher_can_be_assigned_to_a_closed_course() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let teacher = app.add_outsider_teacher("Второй").await;
    close(&app, actor, app.fixture.course.id()).await;

    service(&app)
        .assign_teacher(actor, teacher.id(), app.fixture.course.id())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_student_cannot_be_assigned_as_a_teacher() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .assign_teacher(actor, app.fixture.student.id(), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn an_assigned_admin_gets_the_work_of_the_course() {
    let app = TestApp::start().await;
    let admin_user = app.add_admin().await;
    let actor = Actor::new(admin_user.id());
    let submission = app.queued_submission(app.student(), "pull/1").await;

    service(&app)
        .assign_teacher(actor, admin_user.id(), app.fixture.course.id())
        .await
        .unwrap();

    let card = app
        .application
        .reviews
        .take_next_from_queue(actor, None, Utc::now())
        .await
        .unwrap()
        .expect("the work is in the queue");
    assert_eq!(card.submission.id().as_uuid(), submission);
}

#[tokio::test]
async fn one_of_two_teachers_can_be_removed() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let second = app.add_course_teacher("Второй").await;

    service(&app)
        .unassign_teacher(actor, second.id(), app.fixture.course.id())
        .await
        .unwrap();

    let error = app
        .application
        .courses
        .find_for_teacher(Actor::new(second.id()), app.fixture.course.id())
        .await
        .unwrap_err();
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn the_last_teacher_cannot_be_removed() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .unassign_teacher(actor, app.fixture.teacher.id(), app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::LastCourseTeacher);
}

#[tokio::test]
async fn parallel_removal_of_the_last_two_teachers_keeps_one() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let second = app.add_course_teacher("Второй").await;
    let course = app.course();
    let (mut first_removal, backend) = blocking_transaction(&app).await;
    sqlx::query("select id from teaching_courses where id = $1 for no key update")
        .bind(course)
        .execute(&mut *first_removal)
        .await
        .unwrap();
    sqlx::query(
        "delete from teaching_course_teachers where teaching_course_id = $1 and teacher_id = $2",
    )
    .bind(course)
    .bind(second.id().as_uuid())
    .execute(&mut *first_removal)
    .await
    .unwrap();

    let (removed, ()) = tokio::join!(
        service(&app).unassign_teacher(actor, app.fixture.teacher.id(), app.fixture.course.id()),
        async {
            wait_for_blocked_connection(&app.pool, backend).await;
            first_removal.commit().await.expect("commit first removal");
        }
    );

    assert_eq!(removed.unwrap_err().code(), ErrorCode::LastCourseTeacher);
    let card = service(&app)
        .course_card(actor, app.fixture.course.id())
        .await
        .unwrap();
    assert_eq!(card.teachers.len(), 1);
}

#[tokio::test]
async fn a_change_is_logged_with_ids_and_without_names() {
    let app = TestApp::start().await;
    let admin_user = app.add_admin().await;
    let student = app.fixture.student.id();
    let logs = LogBuffer::default();
    let _capture = logs.capture();

    service(&app)
        .rename_student(
            Actor::new(admin_user.id()),
            student,
            "Секретное Имя".to_owned(),
        )
        .await
        .unwrap();

    let events = logs.events();
    let event = events
        .iter()
        .find(|event| event["fields"]["action"] == "student_renamed")
        .expect("the change is logged");
    assert_eq!(
        event["fields"]["actor_id"],
        admin_user.id().as_uuid().to_string()
    );
    assert_eq!(event["fields"]["target_id"], student.as_uuid().to_string());
    assert!(!logs.text().contains("Секретное"));
}

#[tokio::test]
async fn a_new_group_can_join_a_course() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;

    service(&app)
        .add_course_group(actor, app.fixture.course.id(), group.id())
        .await
        .unwrap();

    let card = service(&app)
        .course_card(actor, app.fixture.course.id())
        .await
        .unwrap();
    assert!(card.details.course.includes_group(group.id()));
}

#[tokio::test]
async fn a_taken_group_code_is_name_taken() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;

    let error = service(&app)
        .rename_group(
            actor,
            group.id(),
            app.fixture.group.code().as_str().to_owned(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NameTaken);
    let stored = service(&app).find_group(actor, group.id()).await.unwrap();
    assert_eq!(stored.code().as_str(), "NEW-GROUP");
}

#[tokio::test]
async fn a_taken_subject_name_is_name_taken_and_keeps_the_old_one() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let subject = add_subject(&app, actor, "Алгоритмы").await;

    let error = service(&app)
        .rename_subject(
            actor,
            subject.id(),
            app.fixture.subject.name().as_str().to_owned(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::NameTaken);
    let stored = service(&app)
        .find_subject(actor, subject.id())
        .await
        .unwrap();
    assert_eq!(stored.subject.name().as_str(), "Алгоритмы");
}

#[tokio::test]
async fn a_created_subject_is_regular() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let subject = add_subject(&app, actor, "Алгоритмы").await;

    let stored = service(&app)
        .find_subject(actor, subject.id())
        .await
        .unwrap();
    assert!(!stored.sandbox);
}

#[tokio::test]
async fn a_year_change_keeps_students_and_enrollments() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let before = enrollments(&app, app.fixture.student.id()).await;

    let group = service(&app)
        .change_group_year(actor, app.fixture.group.id(), 3)
        .await
        .unwrap();

    assert_eq!(group.program().year(), 3);
    assert_eq!(
        stored_user(&app, app.fixture.student.id())
            .await
            .student_group_id(),
        Some(group.id())
    );
    assert_eq!(enrollments(&app, app.fixture.student.id()).await, before);
}

#[tokio::test]
async fn a_year_outside_the_education_level_is_invalid() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "NEW-GROUP").await;

    let error = service(&app)
        .change_group_year(actor, group.id(), 5)
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_pending_notification_keeps_the_subject_name_of_the_review() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let submission = app.queued_submission(app.student(), "pull/1").await;
    app.review(submission, app.teacher(), "accept", "Принято")
        .await
        .ok();

    service(&app)
        .rename_subject(actor, app.fixture.subject.id(), "Новое имя".to_owned())
        .await
        .unwrap();

    let name: String = sqlx::query_scalar("select subject_name from review_notification_outbox")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(name, app.fixture.subject.name().as_str());
}

#[tokio::test]
async fn a_new_course_has_its_groups_and_one_working_invite() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let first = add_group(&app, actor, "G-1").await;
    let second = add_group(&app, actor, "G-2").await;

    let course = add_course(&app, actor, "Алгоритмы", &[&first, &second]).await;

    let card = service(&app).course_card(actor, course.id()).await.unwrap();
    assert_eq!(card.details.groups.len(), 2);
    assert_eq!(
        db::course_invites::find_generation(&app.pool, course.id())
            .await
            .unwrap()
            .map(|generation| generation.as_u32()),
        Some(1)
    );
}

#[tokio::test]
async fn a_group_already_in_the_subject_rolls_the_new_course_back() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let fresh = add_group(&app, actor, "G-1").await;
    let before: i64 = sqlx::query_scalar("select count(*) from teaching_courses")
        .fetch_one(&app.pool)
        .await
        .unwrap();

    let error = service(&app)
        .create_course(new_course(
            &app,
            actor,
            app.fixture.subject.id(),
            &[&fresh, &app.fixture.group],
        ))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::GroupInSubjectCourse);
    let after: i64 = sqlx::query_scalar("select count(*) from teaching_courses")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(after, before);
}

#[tokio::test]
async fn a_group_of_a_closed_course_gets_no_second_course_of_the_subject() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    close(&app, actor, app.fixture.course.id()).await;

    let error = service(&app)
        .create_course(new_course(
            &app,
            actor,
            app.fixture.subject.id(),
            &[&app.fixture.group],
        ))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::GroupInSubjectCourse);
}

#[tokio::test]
async fn a_group_of_another_course_of_the_subject_cannot_be_added() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let foreign = app.add_foreign_course().await;

    let error = service(&app)
        .add_course_group(actor, app.fixture.course.id(), foreign.group.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::GroupInSubjectCourse);
}

#[tokio::test]
async fn a_group_cannot_be_added_to_a_closed_course() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "G-1").await;
    close(&app, actor, app.fixture.course.id()).await;

    let error = service(&app)
        .add_course_group(actor, app.fixture.course.id(), group.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::CourseClosed);
}

#[tokio::test]
async fn a_course_without_groups_is_invalid_input() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .create_course(new_course(&app, actor, app.fixture.subject.id(), &[]))
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn free_groups_exclude_those_in_a_course_of_the_subject() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let free = add_group(&app, actor, "G-FREE").await;

    let page = service(&app)
        .groups_free_for_subject(actor, app.fixture.subject.id(), PageRequest::default())
        .await
        .unwrap();

    let ids: Vec<StudentGroupId> = page.items.iter().map(StudentGroup::id).collect();
    assert_eq!(ids, [free.id()]);
}

#[tokio::test]
async fn the_course_list_splits_working_and_closed_in_subject_order() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let group = add_group(&app, actor, "G-1").await;
    let second = add_course(&app, actor, "Яблоки", &[&group]).await;
    let first = add_course(&app, actor, "Арбузы", &[&group]).await;
    let finished = add_course(&app, actor, "Вишни", &[&group]).await;
    close(&app, actor, finished.id()).await;

    let working = service(&app)
        .list_courses(actor, false, PageRequest::default())
        .await
        .unwrap();
    let closed = service(&app)
        .list_courses(actor, true, PageRequest::default())
        .await
        .unwrap();

    let ids: Vec<_> = working.items.iter().map(|item| item.course.id()).collect();
    assert_eq!(ids, [app.fixture.course.id(), first.id(), second.id()]);
    let ids: Vec<_> = closed.items.iter().map(|item| item.course.id()).collect();
    assert_eq!(ids, [finished.id()]);
}

#[tokio::test]
async fn the_close_preview_counts_published_and_pending() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    app.add_draft_assignment().await;
    app.queued_submission(app.student(), "pull/1").await;

    let preview = service(&app)
        .close_preview(actor, app.fixture.course.id())
        .await
        .unwrap();

    assert_eq!((preview.published, preview.pending), (1, 1));
}

#[tokio::test]
async fn closing_archives_published_assignments_and_keeps_drafts() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let draft = app.add_draft_assignment().await;

    let closed = service(&app)
        .close_course(actor, app.fixture.course.id(), Utc::now())
        .await
        .unwrap();

    assert_eq!(closed.archived, 1);
    assert_eq!(
        assignment_status(&app, app.assignment()).await,
        AssignmentStatus::Archived
    );
    assert_eq!(
        assignment_status(&app, draft.id().as_uuid()).await,
        AssignmentStatus::Draft
    );
}

#[tokio::test]
async fn closing_invalidates_the_issued_invite() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let token = app.course_invite_token().await;

    close(&app, actor, app.fixture.course.id()).await;

    app.register_student(92_000, "Опоздавший", &token)
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
}

#[tokio::test]
async fn pending_work_stays_in_the_queue_after_closing() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let submission = app.queued_submission(app.student(), "pull/1").await;

    let closed = service(&app)
        .close_course(actor, app.fixture.course.id(), Utc::now())
        .await
        .unwrap();

    assert_eq!(closed.pending, 1);
    let body = app.take_next(app.teacher()).await.ok();
    assert_eq!(
        crate::harness::submission_id(&body["submission"]),
        submission
    );
    app.review(submission, app.teacher(), "accept", "Принято")
        .await
        .ok();
}

#[tokio::test]
async fn a_closed_course_keeps_grades_and_archive_for_the_student() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let scored = app.add_scored_assignment(10).await;
    let body = app
        .submit(scored.id().as_uuid(), app.student(), "pull/1")
        .await
        .created();
    app.review_with_score(
        crate::harness::submission_id(&body),
        app.teacher(),
        "accept",
        out_of(8, 10),
        "Принято",
    )
    .await
    .ok();

    close(&app, actor, app.fixture.course.id()).await;

    let archive = app.archived_student_assignments(app.student()).await.ok();
    assert_eq!(archive.as_array().unwrap().len(), 2);
    let grades = app
        .application
        .courses
        .gradebook(
            app.teacher_actor(),
            app.fixture.course.id(),
            app.fixture.group.id(),
            PageRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(grades.items[0].grades.len(), 1);
    app.submit(app.assignment(), app.student(), "pull/2")
        .await
        .error(StatusCode::CONFLICT, "assignment_unavailable");
}

#[tokio::test]
async fn a_sandbox_course_cannot_be_closed() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let sandbox = app.add_sandbox_course().await;

    let error = service(&app)
        .close_course(actor, sandbox.id(), Utc::now())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn closing_twice_reports_course_closed() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    close(&app, actor, app.fixture.course.id()).await;

    let error = service(&app)
        .close_course(actor, app.fixture.course.id(), Utc::now())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::CourseClosed);
}

#[tokio::test]
async fn closing_does_not_deadlock_with_a_registration_in_progress() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let student = app.add_unenrolled_student("Новичок").await;
    let (mut registration, backend) = blocking_transaction(&app).await;
    sqlx::query(
        "select teaching_course_id from teaching_course_invites
         where teaching_course_id = $1 for share",
    )
    .bind(app.course())
    .execute(&mut *registration)
    .await
    .unwrap();

    let admin_service = service(&app).clone();
    let course = app.fixture.course.id();
    let closing =
        tokio::spawn(async move { admin_service.close_course(actor, course, Utc::now()).await });
    wait_for_blocked_connection(&app.pool, backend).await;

    tokio::time::timeout(
        Duration::from_secs(3),
        sqlx::query(
            "insert into teaching_course_students (teaching_course_id, student_id)
             values ($1, $2)",
        )
        .bind(app.course())
        .bind(student.id().as_uuid())
        .execute(&mut *registration),
    )
    .await
    .expect("the foreign key does not wait for closing")
    .expect("enroll during closing");
    registration.commit().await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), closing)
        .await
        .expect("closing finishes")
        .unwrap()
        .expect("closing succeeds");
}

#[tokio::test]
async fn a_reopened_course_is_working_with_a_new_invite() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    let old_token = app.course_invite_token().await;
    close(&app, actor, app.fixture.course.id()).await;

    service(&app)
        .reopen_course(actor, app.fixture.course.id())
        .await
        .unwrap();

    let working = app
        .application
        .courses
        .list_for_teacher(app.teacher_actor(), PageRequest::default())
        .await
        .unwrap();
    assert_eq!(working.items.len(), 1);
    app.register_student(92_001, "Опоздавший", &old_token)
        .await
        .error(StatusCode::FORBIDDEN, "invite_invalid");
    let new_token = app.course_invite_token().await;
    assert_ne!(new_token, old_token);
    app.register_student(92_002, "Вовремя", &new_token)
        .await
        .created();
}

#[tokio::test]
async fn assignments_stay_archived_after_reopening_and_can_be_published() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;
    close(&app, actor, app.fixture.course.id()).await;

    service(&app)
        .reopen_course(actor, app.fixture.course.id())
        .await
        .unwrap();

    assert_eq!(
        assignment_status(&app, app.assignment()).await,
        AssignmentStatus::Archived
    );
    app.publish_assignment(app.assignment(), app.teacher())
        .await
        .ok();
    assert_eq!(
        assignment_status(&app, app.assignment()).await,
        AssignmentStatus::Published
    );
}

#[tokio::test]
async fn reopening_an_open_course_is_invalid_input() {
    let app = TestApp::start().await;
    let actor = admin(&app).await;

    let error = service(&app)
        .reopen_course(actor, app.fixture.course.id())
        .await
        .unwrap_err();

    assert_eq!(error.code(), ErrorCode::InvalidInput);
}
