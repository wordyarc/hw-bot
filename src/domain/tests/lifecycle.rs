use chrono::Duration;

use super::{Fixture, fixture};
use crate::domain::{AssignmentStatus, DomainError, Grading};
use crate::test_support::{admin, assignment, t0, teacher};

#[test]
fn only_teacher_or_admin_can_create_assignment() {
    let Fixture {
        course, student, ..
    } = fixture();

    let result = assignment(&course, &student).try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::PermissionDenied {
            action: "create assignment"
        }
    );
}

#[test]
fn only_course_teacher_or_admin_can_publish_assignment() {
    let Fixture {
        teacher: course_teacher,
        course,
        ..
    } = fixture();
    let other_teacher = teacher(2);
    let admin = admin(3);
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
    let Fixture {
        teacher, course, ..
    } = fixture();
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
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut assignment = assignment(&course, &teacher).published().build();
    assignment.archive(&course, &teacher).unwrap();

    let reopened_at = t0() + Duration::days(180);
    assignment.publish(&course, &teacher, reopened_at).unwrap();

    assert_eq!(assignment.status(), AssignmentStatus::Published);
    assert_eq!(assignment.published_at(), Some(reopened_at));
}

#[test]
fn a_reopened_assignment_accepts_a_new_deadline() {
    let Fixture {
        teacher, course, ..
    } = fixture();
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
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut draft = assignment(&course, &teacher).build();
    let mut published = assignment(&course, &teacher).published().build();

    draft.archive(&course, &teacher).unwrap();
    published.archive(&course, &teacher).unwrap();

    assert_eq!(draft.status(), AssignmentStatus::Archived);
    assert_eq!(published.status(), AssignmentStatus::Archived);
}

#[test]
fn assignment_can_be_archived_only_once() {
    let Fixture {
        teacher, course, ..
    } = fixture();
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
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut assignment = assignment(&course, &teacher).build();
    let deadline_at = assignment.deadline_at();
    assignment.archive(&course, &teacher).unwrap();

    let result = assignment.change_deadline(&course, &teacher, deadline_at + Duration::days(1));

    assert_eq!(result.unwrap_err(), DomainError::AssignmentIsArchived);
    assert_eq!(assignment.deadline_at(), deadline_at);
}

#[test]
fn archived_assignment_keeps_its_grading() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut assignment = assignment(&course, &teacher).scored(5).build();
    assignment.archive(&course, &teacher).unwrap();

    let result = assignment.change_grading(&course, &teacher, Grading::PassFail, None);

    assert_eq!(result.unwrap_err(), DomainError::AssignmentIsArchived);
    assert_eq!(assignment.grading(), Grading::scored(5).unwrap());
}

#[test]
fn course_teacher_or_admin_can_move_deadline_both_ways() {
    let Fixture {
        teacher: course_teacher,
        course,
        ..
    } = fixture();
    let (other_teacher, admin) = (teacher(2), admin(3));
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
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut assignment = assignment(&course, &teacher).build();
    let created_at = assignment.created_at();

    let result = assignment.change_deadline(&course, &teacher, created_at - Duration::seconds(1));

    assert_eq!(
        result.unwrap_err(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt
    );
}

#[test]
fn a_deadline_must_lie_strictly_after_the_creation_moment() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let just_after = t0() + Duration::nanoseconds(1);

    assert_eq!(
        assignment(&course, &teacher)
            .created_at(t0())
            .deadline_at(t0())
            .try_build()
            .unwrap_err(),
        DomainError::AssignmentDeadlineMustBeAfterCreatedAt
    );
    assert_eq!(
        assignment(&course, &teacher)
            .created_at(t0())
            .deadline_at(just_after)
            .try_build()
            .map(|assignment| assignment.deadline_at()),
        Ok(just_after)
    );
}
