use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DomainError, Role, StaffRole,
    StudentAssignmentProgress, Submission, SubmissionHistory, SubmissionId, SubmissionStatus,
    TelegramUserId, User, UserId,
};
use crate::test_support::{
    ASSIGNMENT_BODY, ASSIGNMENT_TITLE, GROUP_CODE, OTHER_GROUP_CODE, PULL_REQUEST_URL, admin,
    assignment, course_for, history, student, student_group, submission, t0, teacher,
};

fn reviewed_at() -> DateTime<Utc> {
    t0() + Duration::hours(2)
}

#[test]
fn only_teacher_or_admin_can_create_assignment() {
    let teacher = teacher(10);
    let group = student_group(GROUP_CODE);
    let student = student(1, &group);
    let course = course_for(&group, &teacher);

    let result = Assignment::new(
        AssignmentId::new(),
        &course,
        &student,
        ASSIGNMENT_TITLE,
        ASSIGNMENT_BODY,
        2,
        t0(),
        t0() + Duration::days(7),
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
    let course_teacher = teacher(1);
    let other_teacher = teacher(2);
    let admin = admin(3);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &course_teacher);
    let mut assignment = assignment(&course, &course_teacher).build();

    let result = assignment.publish(&course, &other_teacher, t0());

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "publish assignment"
        }
    );

    assignment.publish(&course, &admin, t0()).unwrap();
}

#[test]
fn assignment_can_be_published_only_once() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).build();

    assignment.publish(&course, &teacher, t0()).unwrap();

    assert_eq!(
        assignment.publish(&course, &teacher, t0()).unwrap_err(),
        DomainError::AssignmentStatusTransitionNotAllowed {
            from: AssignmentStatus::Published,
            to: AssignmentStatus::Published,
        }
    );
}

#[test]
fn archived_assignment_can_be_published_again_for_a_retake() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).published().build();
    assignment.archive(&course, &teacher).unwrap();

    let reopened_at = t0() + Duration::days(180);
    assignment.publish(&course, &teacher, reopened_at).unwrap();

    assert_eq!(assignment.status(), AssignmentStatus::Published);
    assert_eq!(assignment.published_at(), Some(reopened_at));
}

#[test]
fn a_reopened_assignment_accepts_a_new_deadline() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).published().build();
    assignment.archive(&course, &teacher).unwrap();
    assignment.publish(&course, &teacher, t0()).unwrap();

    let retake_deadline = t0() + Duration::days(200);
    assignment
        .change_deadline(&course, &teacher, retake_deadline)
        .unwrap();

    assert_eq!(assignment.deadline_at(), retake_deadline);
}

#[test]
fn both_draft_and_published_assignments_can_be_archived() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut draft = assignment(&course, &teacher).build();
    let mut published = assignment(&course, &teacher).published().build();

    draft.archive(&course, &teacher).unwrap();
    published.archive(&course, &teacher).unwrap();

    assert_eq!(draft.status(), AssignmentStatus::Archived);
    assert_eq!(published.status(), AssignmentStatus::Archived);
}

#[test]
fn assignment_can_be_archived_only_once() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).build();
    assignment.archive(&course, &teacher).unwrap();

    assert_eq!(
        assignment.archive(&course, &teacher).unwrap_err(),
        DomainError::AssignmentStatusTransitionNotAllowed {
            from: AssignmentStatus::Archived,
            to: AssignmentStatus::Archived,
        }
    );
}

#[test]
fn archived_assignment_keeps_its_deadline() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).build();
    let deadline_at = assignment.deadline_at();
    assignment.archive(&course, &teacher).unwrap();

    let result = assignment.change_deadline(&course, &teacher, deadline_at + Duration::days(1));

    assert_eq!(result.unwrap_err(), DomainError::AssignmentIsArchived);
    assert_eq!(assignment.deadline_at(), deadline_at);
}

#[test]
fn added_course_teacher_can_review_and_manage_assignment() {
    let course_teacher = teacher(1);
    let reviewer = teacher(2);
    let group = student_group(GROUP_CODE);
    let mut course = course_for(&group, &course_teacher);
    course.add_teacher(&course_teacher, &reviewer).unwrap();
    let student = student(3, &group);
    let assignment = assignment(&course, &course_teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .accept(&assignment, &course, &reviewer, "95/100", reviewed_at())
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Accepted);
    assert_eq!(submission.reviewed_by(), Some(reviewer.id()));
}

#[test]
fn student_can_submit_published_assignment_until_attempts_are_exhausted() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher)
        .max_attempts(2)
        .published()
        .build();

    let first = submission(&assignment, &student).build();

    assert_eq!(first.attempt_number(), 1);
    assert_eq!(first.status(), SubmissionStatus::PendingReview);

    let exhausted = submission(&assignment, &student)
        .history(history().used(2).build())
        .try_build();

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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let deadline_at = assignment.deadline_at();
    let submitted_at = deadline_at + Duration::seconds(1);

    let result = submission(&assignment, &student)
        .submitted_at(submitted_at)
        .try_build();

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
    let (course_teacher, other_teacher, admin) = (teacher(1), teacher(2), admin(3));
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &course_teacher);
    let mut assignment = assignment(&course, &course_teacher).build();
    let created_at = assignment.created_at();

    let earlier_deadline = created_at + Duration::days(3);
    assignment
        .change_deadline(&course, &course_teacher, earlier_deadline)
        .unwrap();
    assert_eq!(assignment.deadline_at(), earlier_deadline);

    let later_deadline = created_at + Duration::days(14);
    assignment
        .change_deadline(&course, &admin, later_deadline)
        .unwrap();
    assert_eq!(assignment.deadline_at(), later_deadline);

    let result = assignment.change_deadline(&course, &other_teacher, earlier_deadline);
    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "change assignment deadline"
        }
    );
}

#[test]
fn deadline_cannot_be_moved_before_creation() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let mut assignment = assignment(&course, &teacher).build();
    let created_at = assignment.created_at();

    let result = assignment.change_deadline(&course, &teacher, created_at - Duration::seconds(1));

    assert_eq!(
        result.unwrap_err(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt
    );
}

#[test]
fn student_from_another_group_cannot_submit_assignment() {
    let teacher = teacher(1);
    let assignment_group = student_group(GROUP_CODE);
    let other_group = student_group(OTHER_GROUP_CODE);
    let course = course_for(&assignment_group, &teacher);
    let student = student(2, &other_group);
    let assignment = assignment(&course, &teacher).published().build();

    let result = submission(&assignment, &student).try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::StudentDoesNotBelongToAssignmentGroup
    );
}

#[test]
fn draft_assignment_cannot_be_submitted() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let draft = assignment(&course, &teacher).build();

    let result = submission(&draft, &student).try_build();

    assert_eq!(result.unwrap_err(), DomainError::AssignmentIsNotPublished);
}

#[test]
fn only_course_teacher_or_admin_can_grant_extra_attempts() {
    let course_teacher = teacher(1);
    let other_teacher = teacher(2);
    let admin = admin(3);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &course_teacher);
    let student = student(4, &group);
    let assignment = assignment(&course, &course_teacher)
        .max_attempts(1)
        .published()
        .build();

    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();
    let result = progress.grant_extra_attempts(&assignment, &course, &other_teacher, 1);

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "grant extra attempts"
        }
    );

    progress
        .grant_extra_attempts(&assignment, &course, &admin, 1)
        .unwrap();
    assert_eq!(progress.extra_attempts(), 1);
}

#[test]
fn extra_attempts_extend_submission_limit() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher)
        .max_attempts(1)
        .published()
        .build();

    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();
    progress
        .grant_extra_attempts(&assignment, &course, &teacher, 1)
        .unwrap();

    let second_attempt = submission(&assignment, &student)
        .history(history().used(1).extra(progress.extra_attempts()).build())
        .build();

    assert_eq!(second_attempt.attempt_number(), 2);
}

#[test]
fn accepted_assignment_cannot_be_submitted_again() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher)
        .max_attempts(3)
        .published()
        .build();

    let result = submission(&assignment, &student)
        .history(history().used(1).accepted().build())
        .try_build();

    assert_eq!(result.unwrap_err(), DomainError::SubmissionAlreadyAccepted);
}

#[test]
fn pending_submission_blocks_next_attempt() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher)
        .max_attempts(3)
        .published()
        .build();

    let result = submission(&assignment, &student)
        .history(history().pending().build())
        .try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::SubmissionAlreadyPendingReview
    );
}

#[test]
fn accepted_review_requires_comment_and_stores_it() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    let empty_comment = submission.accept(&assignment, &course, &teacher, " ", reviewed_at());

    assert_eq!(
        empty_comment.unwrap_err(),
        DomainError::EmptyField {
            field: "submission.review_comment"
        }
    );
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);

    submission
        .accept(&assignment, &course, &teacher, "95/100", reviewed_at())
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Accepted);
    assert_eq!(submission.review_comment().unwrap().as_str(), "95/100");
}

#[test]
fn teacher_can_review_pending_submission_once() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .reject(
            &assignment,
            &course,
            &teacher,
            "Add tests for parser errors.",
            reviewed_at(),
        )
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Rejected);
    assert_eq!(submission.reviewed_by(), Some(teacher.id()));
    assert_eq!(
        submission.review_comment().unwrap().as_str(),
        "Add tests for parser errors."
    );

    let second_review = submission.accept(&assignment, &course, &teacher, "95/100", reviewed_at());
    assert_eq!(
        second_review.unwrap_err(),
        DomainError::SubmissionIsNotPendingReview
    );
}

#[test]
fn only_course_teacher_or_admin_can_review_submission() {
    let course_teacher = teacher(1);
    let other_teacher = teacher(2);
    let admin = admin(3);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &course_teacher);
    let student = student(4, &group);
    let assignment = assignment(&course, &course_teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(
        &assignment,
        &course,
        &other_teacher,
        "Accepted, 95/100",
        reviewed_at(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "review homework"
        }
    );

    submission
        .reject(&assignment, &course, &admin, "Fix CI", reviewed_at())
        .unwrap();
}

#[test]
fn student_cannot_review_submission() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(&assignment, &course, &student, "95/100", reviewed_at());

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "review homework"
        }
    );
}

#[test]
fn submission_of_another_assignment_cannot_be_reviewed() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let (own, other) = (
        assignment(&course, &teacher).published().build(),
        assignment(&course, &teacher).published().build(),
    );
    let mut submission = submission(&own, &student).build();

    let result = submission.accept(&other, &course, &teacher, "95/100", reviewed_at());

    assert_eq!(
        result.unwrap_err(),
        DomainError::SubmissionDoesNotBelongToAssignment
    );
}

#[test]
fn only_admin_can_register_staff() {
    let admin = admin(1);
    let teacher = teacher(2);
    let group = student_group(GROUP_CODE);
    let student = student(3, &group);

    for actor in [&teacher, &student] {
        assert_eq!(
            User::new_staff(
                UserId::new(),
                TelegramUserId::new(100).unwrap(),
                StaffRole::Teacher,
                "New teacher",
                actor,
                t0(),
            )
            .unwrap_err(),
            DomainError::PermissionDenied {
                action: "register staff"
            }
        );
    }

    let registered = User::new_staff(
        UserId::new(),
        TelegramUserId::new(100).unwrap(),
        StaffRole::Teacher,
        "New teacher",
        &admin,
        t0(),
    )
    .unwrap();

    assert_eq!(registered.role(), Role::Teacher);
    assert_eq!(
        registered.student_group_id(),
        None,
        "staff is never bound to a student group"
    );
}

#[test]
fn submission_content_is_trimmed() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();

    let submission = Submission::submit(
        SubmissionId::new(),
        &assignment,
        &student,
        SubmissionHistory::default(),
        format!("  {PULL_REQUEST_URL}  "),
        t0(),
    )
    .unwrap();

    assert_eq!(submission.content().as_str(), PULL_REQUEST_URL);
}

#[test]
fn remaining_attempts_count_the_granted_ones() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let assignment = assignment(&course, &teacher).max_attempts(2).build();

    let history = history().used(2).extra(1).build();

    assert_eq!(history.allowed_attempts_for(&assignment), 3);
    assert_eq!(history.remaining_attempts_for(&assignment), 1);
}

#[test]
fn remaining_attempts_stop_at_zero() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let assignment = assignment(&course, &teacher).max_attempts(1).build();

    let history = history().used(2).build();

    assert_eq!(history.remaining_attempts_for(&assignment), 0);
}

#[test]
fn extra_attempts_cannot_exceed_the_storage_limit() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher)
        .max_attempts(crate::domain::MAX_ATTEMPTS - 1)
        .build();
    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();

    assert_eq!(
        progress
            .grant_extra_attempts(&assignment, &course, &teacher, 2)
            .unwrap_err(),
        DomainError::AttemptLimitExceeded {
            value: u32::from(crate::domain::MAX_ATTEMPTS) + 1,
            max: crate::domain::MAX_ATTEMPTS,
        }
    );
}
