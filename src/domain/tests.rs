use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    Assignment, AssignmentId, DomainError, EducationLevel, ExtraAttemptGrant, ExtraAttemptGrantId,
    Role, StudentGroup, StudentGroupId, StudyProgram, Subject, SubjectId, Submission,
    SubmissionHistory, SubmissionId, SubmissionStatus, TeachingCourse, TeachingCourseId,
    TelegramUserId, User, UserId,
};

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn user(telegram_id: i64, role: Role) -> User {
    User::new(
        UserId::new(),
        TelegramUserId::new(telegram_id).unwrap(),
        role,
        format!("{role:?}"),
        now(),
    )
    .unwrap()
}

fn student(telegram_id: i64, student_group: &StudentGroup) -> User {
    User::new_student(
        UserId::new(),
        TelegramUserId::new(telegram_id).unwrap(),
        "Student",
        student_group,
        now(),
    )
    .unwrap()
}

fn student_group(code: &str) -> StudentGroup {
    StudentGroup::new(
        StudentGroupId::new(),
        code,
        StudyProgram::new(EducationLevel::Bachelor, 1).unwrap(),
    )
    .unwrap()
}

fn teaching_course(teacher: &User, student_group: &StudentGroup) -> TeachingCourse {
    let subject = Subject::new(SubjectId::new(), "Backend development").unwrap();

    TeachingCourse::new(
        TeachingCourseId::new(),
        &subject,
        student_group,
        teacher,
        now(),
    )
    .unwrap()
}

fn published_assignment(
    teacher: &User,
    teaching_course: &TeachingCourse,
    max_attempts: u16,
) -> Assignment {
    let created_at = now();
    let mut assignment = Assignment::new(
        AssignmentId::new(),
        teaching_course,
        teacher,
        "Rust ownership",
        "Send a pull request link.",
        max_attempts,
        created_at,
        created_at + Duration::days(7),
    )
    .unwrap();

    assignment.publish(teacher, now()).unwrap();
    assignment
}

#[test]
fn only_teacher_or_admin_can_create_assignment() {
    let teacher = user(10, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let student = student(1, &group);
    let course = teaching_course(&teacher, &group);

    let result = Assignment::new(
        AssignmentId::new(),
        &course,
        &student,
        "Rust ownership",
        "Send a pull request link.",
        2,
        now(),
        now() + Duration::days(7),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "create assignment"
        }
    );
}

#[test]
fn only_course_teacher_or_admin_can_publish_assignment() {
    let course_teacher = user(1, Role::Teacher);
    let other_teacher = user(2, Role::Teacher);
    let admin = user(3, Role::Admin);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&course_teacher, &group);
    let created_at = now();
    let mut assignment = Assignment::new(
        AssignmentId::new(),
        &course,
        &course_teacher,
        "Rust ownership",
        "Send a pull request link.",
        2,
        created_at,
        created_at + Duration::days(7),
    )
    .unwrap();

    let result = assignment.publish(&other_teacher, now());

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "publish assignment"
        }
    );

    assignment.publish(&admin, now()).unwrap();
}

#[test]
fn student_can_submit_published_assignment_until_attempts_are_exhausted() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 2);

    let first = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::new(0, 0, false, false),
        "https://github.com/course/hw/pull/1",
        now(),
    )
    .unwrap();

    assert_eq!(first.attempt_number(), 1);
    assert_eq!(first.status(), SubmissionStatus::PendingReview);

    let exhausted = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::new(2, 0, false, false),
        "https://github.com/course/hw/pull/2",
        now(),
    );

    assert_eq!(
        exhausted.unwrap_err(),
        DomainError::AttemptsExhausted {
            allowed: 2,
            used: 2
        }
    );
}

#[test]
fn student_cannot_submit_assignment_after_deadline() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let created_at = now();
    let deadline_at = created_at + Duration::days(7);
    let mut assignment = Assignment::new(
        AssignmentId::new(),
        &course,
        &teacher,
        "Rust ownership",
        "Send a pull request link.",
        2,
        created_at,
        deadline_at,
    )
    .unwrap();
    assignment.publish(&teacher, created_at).unwrap();
    let submitted_at = deadline_at + Duration::seconds(1);

    let result = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/2",
        submitted_at,
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::AssignmentDeadlineExceeded {
            deadline_at,
            submitted_at,
        }
    );
}

#[test]
fn course_teacher_or_admin_can_move_deadline_both_ways() {
    let teacher = user(1, Role::Teacher);
    let other_teacher = user(2, Role::Teacher);
    let admin = user(3, Role::Admin);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let created_at = now();
    let initial_deadline = created_at + Duration::days(7);
    let mut assignment = Assignment::new(
        AssignmentId::new(),
        &course,
        &teacher,
        "Rust ownership",
        "Send a pull request link.",
        2,
        created_at,
        initial_deadline,
    )
    .unwrap();

    let earlier_deadline = created_at + Duration::days(3);
    assignment
        .change_deadline(&teacher, earlier_deadline)
        .unwrap();
    assert_eq!(assignment.deadline_at(), earlier_deadline);

    let later_deadline = created_at + Duration::days(14);
    assignment.change_deadline(&admin, later_deadline).unwrap();
    assert_eq!(assignment.deadline_at(), later_deadline);

    let result = assignment.change_deadline(&other_teacher, initial_deadline);
    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "change assignment deadline"
        }
    );
}

#[test]
fn student_from_another_group_cannot_submit_assignment() {
    let teacher = user(1, Role::Teacher);
    let assignment_group = student_group("B9122-02.03.21");
    let other_group = student_group("M9241-09.04.03");
    let course = teaching_course(&teacher, &assignment_group);
    let student = student(2, &other_group);
    let assignment = published_assignment(&teacher, &course, 2);

    let result = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/2",
        now(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::StudentDoesNotBelongToAssignmentGroup
    );
}

#[test]
fn study_program_validates_year_by_education_level() {
    assert_eq!(
        StudyProgram::new(EducationLevel::Master, 3).unwrap_err(),
        DomainError::InvalidStudyYear {
            education_level: "master",
            value: 3,
            max: 2,
        }
    );
}

#[test]
fn only_course_teacher_or_admin_can_grant_extra_attempts() {
    let course_teacher = user(1, Role::Teacher);
    let other_teacher = user(2, Role::Teacher);
    let admin = user(3, Role::Admin);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&course_teacher, &group);
    let student = student(4, &group);
    let assignment = published_assignment(&course_teacher, &course, 1);

    let result = ExtraAttemptGrant::new(
        ExtraAttemptGrantId::new(),
        &assignment,
        &student,
        &other_teacher,
        1,
        None,
        now(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "grant extra attempts"
        }
    );

    ExtraAttemptGrant::new(
        ExtraAttemptGrantId::new(),
        &assignment,
        &student,
        &admin,
        1,
        None,
        now(),
    )
    .unwrap();
}

#[test]
fn extra_attempt_grant_extends_submission_limit() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 1);

    let grant = ExtraAttemptGrant::new(
        ExtraAttemptGrantId::new(),
        &assignment,
        &student,
        &teacher,
        1,
        Some("Fixed CI setup".to_owned()),
        now(),
    )
    .unwrap();

    let second_attempt = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::new(1, grant.amount(), false, false),
        "https://github.com/course/hw/pull/3",
        now(),
    )
    .unwrap();

    assert_eq!(second_attempt.attempt_number(), 2);
}

#[test]
fn accepted_assignment_cannot_be_submitted_again() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 3);

    let result = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::new(1, 0, true, false),
        "https://github.com/course/hw/pull/4",
        now(),
    );

    assert_eq!(result.unwrap_err(), DomainError::SubmissionAlreadyAccepted);
}

#[test]
fn pending_submission_blocks_next_attempt() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 3);

    let result = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::new(0, 0, false, true),
        "https://github.com/course/hw/pull/4",
        now(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::SubmissionAlreadyPendingReview
    );
}

#[test]
fn accepted_review_requires_comment_and_stores_it() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 2);
    let mut submission = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/1",
        now(),
    )
    .unwrap();

    let empty_comment = submission.accept(&assignment, &teacher, " ", now());

    assert_eq!(
        empty_comment.unwrap_err(),
        DomainError::EmptyField {
            field: "submission.review_comment"
        }
    );
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);

    submission
        .accept(&assignment, &teacher, "95/100", now())
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Accepted);
    assert_eq!(submission.review_comment().unwrap().as_str(), "95/100");
}

#[test]
fn teacher_can_review_pending_submission_once() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 2);
    let mut submission = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/1",
        now(),
    )
    .unwrap();

    submission
        .reject(&assignment, &teacher, "Add tests for parser errors.", now())
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Rejected);
    assert_eq!(submission.reviewed_by(), Some(teacher.id()));
    assert_eq!(
        submission.review_comment().unwrap().as_str(),
        "Add tests for parser errors."
    );

    let second_review = submission.accept(&assignment, &teacher, "95/100", now());
    assert_eq!(
        second_review.unwrap_err(),
        DomainError::SubmissionIsNotPendingReview
    );
}

#[test]
fn only_course_teacher_or_admin_can_review_submission() {
    let course_teacher = user(1, Role::Teacher);
    let other_teacher = user(2, Role::Teacher);
    let admin = user(3, Role::Admin);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&course_teacher, &group);
    let student = student(4, &group);
    let assignment = published_assignment(&course_teacher, &course, 2);
    let mut submission = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/1",
        now(),
    )
    .unwrap();

    let result = submission.accept(&assignment, &other_teacher, "Accepted, 95/100", now());

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "review homework"
        }
    );

    submission
        .reject(&assignment, &admin, "Fix CI", now())
        .unwrap();
}

#[test]
fn student_cannot_review_submission() {
    let teacher = user(1, Role::Teacher);
    let group = student_group("B9122-02.03.21");
    let course = teaching_course(&teacher, &group);
    let student = student(2, &group);
    let assignment = published_assignment(&teacher, &course, 2);
    let mut submission = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        "https://github.com/course/hw/pull/1",
        now(),
    )
    .unwrap();

    let result = submission.accept(&assignment, &student, "95/100", now());

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "review homework"
        }
    );
}
