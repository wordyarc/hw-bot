use chrono::Duration;
use proptest::prelude::*;

use super::{Fixture, fixture};
use crate::domain::{
    AssignmentStatus, DeductedFrom, DomainError, MAX_ATTEMPTS, StudentAssignmentProgress,
    SubmissionHistory, SubmissionStatus,
};
use crate::test_support::{admin, assignment, course_for, history, submission, t0, teacher};

#[test]
fn student_can_submit_published_assignment_until_attempts_are_exhausted() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
fn staff_receive_no_extra_attempts() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();

    for staff in [&teacher, &admin(2)] {
        assert_eq!(
            StudentAssignmentProgress::new(&assignment, staff, 0).unwrap_err(),
            DomainError::PermissionDenied {
                action: "receive extra attempts"
            },
            "{:?}",
            staff.role()
        );
    }
}

#[test]
fn only_course_teacher_or_admin_can_grant_extra_attempts() {
    let Fixture {
        teacher: course_teacher,
        course,
        student,
        ..
    } = fixture();
    let other_teacher = teacher(2);
    let admin = admin(3);
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
fn remaining_attempts_count_the_granted_ones() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let assignment = assignment(&course, &teacher).max_attempts(2).build();

    let history = history().used(2).extra(1).build();

    assert_eq!(history.allowed_attempts_for(&assignment), 3);
    assert_eq!(history.remaining_attempts_for(&assignment), 1);
}

#[test]
fn remaining_attempts_stop_at_zero() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let assignment = assignment(&course, &teacher).max_attempts(1).build();

    let history = history().used(2).build();

    assert_eq!(history.remaining_attempts_for(&assignment), 0);
}

fn histories() -> impl Strategy<Value = SubmissionHistory> {
    let rarely = || prop::bool::weighted(0.15);
    (0..=3u16, 0..=2u16, 0..=2u16, rarely(), rarely(), rarely()).prop_map(
        |(used, makeups, extra, accepted, accepted_makeup, pending)| {
            let mut history = history()
                .used(used)
                .submissions(used + makeups)
                .extra(extra);
            if accepted {
                history = history.accepted();
            }
            if accepted_makeup {
                history = history.accepted_makeup();
            }
            if pending {
                history = history.pending();
            }
            history.build()
        },
    )
}

fn statuses() -> impl Strategy<Value = AssignmentStatus> {
    prop_oneof![
        3 => Just(AssignmentStatus::Published),
        1 => Just(AssignmentStatus::Draft),
        1 => Just(AssignmentStatus::Archived),
    ]
}

fn around_closing() -> impl Strategy<Value = Duration> {
    prop_oneof![
        4 => (-10 * 24 * 60..=4 * 24 * 60i64).prop_map(Duration::minutes),
        1 => Just(-Duration::nanoseconds(1)),
        1 => Just(Duration::zero()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn a_new_attempt_is_offered_exactly_when_a_regular_submission_is_admitted(
        status in statuses(),
        max_attempts in 1..=3u16,
        late in any::<bool>(),
        history in histories(),
        offset in around_closing(),
    ) {
        let Fixture { teacher, course, student, .. } = fixture();
        let mut work = assignment(&course, &teacher).max_attempts(max_attempts);
        if late {
            work = work.scored(10).late_policy(7, 1, 2, DeductedFrom::WorkScore);
        }
        if status != AssignmentStatus::Draft {
            work = work.published();
        }
        let mut work = work.build();
        if status == AssignmentStatus::Archived {
            work.archive(&course, &teacher).unwrap();
        }
        let at = work.closes_at() + offset;

        let submitted = submission(&work, &student)
            .history(history)
            .submitted_at(at)
            .try_build();

        prop_assert_eq!(
            history.accepts_new_attempt_for(&work, at),
            submitted.is_ok(),
            "{:?}",
            submitted
        );
    }
}

#[test]
fn a_regular_attempt_is_refused_by_the_work_state_or_a_closed_window() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let draft = assignment(&course, &teacher).max_attempts(2).build();
    let assignment = assignment(&course, &teacher)
        .max_attempts(2)
        .published()
        .build();
    let (open, closed) = (t0(), assignment.closes_at());

    assert_eq!(
        history()
            .used(1)
            .build()
            .ensure_regular_attempt(&assignment, open),
        Ok(())
    );
    for (history, at, refusal) in [
        (
            history().used(2).build(),
            open,
            DomainError::AttemptsExhausted {
                allowed: 2,
                used: 2,
            },
        ),
        (
            history().accepted().build(),
            open,
            DomainError::SubmissionAlreadyAccepted,
        ),
        (
            history().pending().build(),
            open,
            DomainError::SubmissionAlreadyPendingReview,
        ),
        (
            history().build(),
            closed,
            DomainError::AssignmentDeadlineExceeded {
                deadline_at: assignment.deadline_at(),
                submitted_at: closed,
            },
        ),
    ] {
        assert_eq!(
            history.ensure_regular_attempt(&assignment, at),
            Err(refusal)
        );
    }
    assert_eq!(
        history().build().ensure_regular_attempt(&draft, open),
        Err(DomainError::AssignmentIsNotPublished)
    );
}

#[test]
fn a_refused_attempt_names_pending_then_accepted_work_before_the_closed_window() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let assignment = assignment(&course, &teacher).scored(10).published().build();
    let closed = assignment.closes_at();

    assert_eq!(
        history()
            .accepted_with(5)
            .pending()
            .build()
            .ensure_regular_attempt(&assignment, closed),
        Err(DomainError::SubmissionAlreadyPendingReview)
    );
    assert_eq!(
        history()
            .accepted_with(5)
            .build()
            .ensure_regular_attempt(&assignment, closed),
        Err(DomainError::SubmissionAlreadyAccepted)
    );
}

#[test]
fn extra_attempts_of_another_assignment_are_refused() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let another = assignment(&course, &teacher).build();
    let assignment = assignment(&course, &teacher).build();
    let mut progress = StudentAssignmentProgress::new(&another, &student, 0).unwrap();

    assert_eq!(
        progress
            .grant_extra_attempts(&assignment, &course, &teacher, 1)
            .unwrap_err(),
        DomainError::StudentProgressDoesNotBelongToAssignment
    );
}

#[test]
fn extra_attempts_are_refused_through_a_foreign_course() {
    let Fixture {
        teacher,
        group,
        course,
        student,
    } = fixture();
    let foreign_course = course_for(&group, &teacher);
    let assignment = assignment(&course, &teacher).build();
    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();

    assert_eq!(
        progress
            .grant_extra_attempts(&assignment, &foreign_course, &teacher, 1)
            .unwrap_err(),
        DomainError::AssignmentDoesNotBelongToTeachingCourse
    );
}

#[test]
fn granting_zero_extra_attempts_is_rejected() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).build();
    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();

    assert_eq!(
        progress
            .grant_extra_attempts(&assignment, &course, &teacher, 0)
            .unwrap_err(),
        DomainError::InvalidAttemptGrant { value: 0 }
    );
}

#[test]
fn extra_attempts_cannot_exceed_the_storage_limit() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher)
        .max_attempts(MAX_ATTEMPTS - 1)
        .build();
    let mut progress = StudentAssignmentProgress::new(&assignment, &student, 0).unwrap();

    assert_eq!(
        progress
            .grant_extra_attempts(&assignment, &course, &teacher, 2)
            .unwrap_err(),
        DomainError::AttemptLimitExceeded {
            value: u32::from(MAX_ATTEMPTS) + 1,
            max: MAX_ATTEMPTS,
        }
    );
}
