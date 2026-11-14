use chrono::{DateTime, Duration, Utc};

use super::{Fixture, fixture, reviewed_at, weekly_penalty_assignment};
use crate::domain::{
    Assignment, CourseMakeup, DomainError, MakeupPool, MakeupRequest, Resubmission, Submission,
    SubmissionHistory, SubmissionStatus, TeachingCourse, User,
};
use crate::test_support::{ReviewShortcuts, assignment, history, student, submission, t0};

fn withdrawn_at() -> DateTime<Utc> {
    t0() + Duration::hours(3)
}

#[test]
fn the_author_withdraws_a_pending_submission() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();
    let submitted_at = submission.submitted_at();

    submission.withdraw(&student, withdrawn_at()).unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Withdrawn);
    assert_eq!(submission.withdrawn_at(), Some(withdrawn_at()));
    assert_eq!(submission.submitted_at(), submitted_at);
    assert!(submission.reviewed_at().is_none());
}

#[test]
fn only_the_author_withdraws_a_submission() {
    let Fixture {
        teacher,
        group,
        course,
        student: author,
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &author).build();

    for other in [student(3, &group), teacher] {
        assert_eq!(
            submission.withdraw(&other, withdrawn_at()),
            Err(DomainError::PermissionDenied {
                action: "withdraw submission"
            })
        );
    }
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

#[test]
fn a_withdrawn_submission_is_not_withdrawn_again() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();
    submission.withdraw(&student, withdrawn_at()).unwrap();

    let again = submission.withdraw(&student, withdrawn_at() + Duration::minutes(1));

    assert_eq!(again, Err(DomainError::SubmissionAlreadyWithdrawn));
    assert_eq!(submission.withdrawn_at(), Some(withdrawn_at()));
}

#[test]
fn a_reviewed_submission_is_not_withdrawn() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let mut accepted = submission(&assignment, &student).build();
    accepted
        .accept(&assignment, &course, &teacher, None, "ok", reviewed_at())
        .unwrap();
    let mut rejected = submission(&assignment, &student).build();
    rejected
        .reject(&assignment, &course, &teacher, "нет", reviewed_at())
        .unwrap();

    for reviewed in [&mut accepted, &mut rejected] {
        let status = reviewed.status();

        assert_eq!(
            reviewed.withdraw(&student, withdrawn_at()),
            Err(DomainError::SubmissionIsNotPendingReview)
        );
        assert_eq!(reviewed.status(), status);
    }
}

#[test]
fn a_withdrawn_submission_is_not_reviewed() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();
    submission.withdraw(&student, withdrawn_at()).unwrap();

    let review = submission.reject(&assignment, &course, &teacher, "нет", reviewed_at());

    assert_eq!(review, Err(DomainError::SubmissionAlreadyWithdrawn));
    assert_eq!(submission.status(), SubmissionStatus::Withdrawn);
}

fn pending() -> SubmissionHistory {
    history().submissions(1).pending().build()
}

fn makeup_window(attempts: u16, opened_at: DateTime<Utc>) -> CourseMakeup {
    CourseMakeup::open(t0() + Duration::days(60), attempts, opened_at).unwrap()
}

fn credit_assignment(course: &TeachingCourse, teacher: &User) -> Assignment {
    assignment(course, teacher)
        .scored(10)
        .makeup_credit(6)
        .published()
        .build()
}

fn after_closing(assignment: &Assignment) -> DateTime<Utc> {
    assignment.closes_at() + Duration::days(1)
}

fn pending_makeup(
    assignment: &Assignment,
    student: &User,
    window: CourseMakeup,
    used: u16,
) -> (Submission, MakeupPool) {
    let submitted_at = after_closing(assignment);
    let makeup = submission(assignment, student)
        .submitted_at(submitted_at)
        .makeup(MakeupRequest {
            course: Some(window),
            consent: true,
            balance: window.balance(0, used),
        })
        .build();
    let pool = MakeupPool {
        course: window,
        balance: window.balance(0, used + 1),
    };

    (makeup, pool)
}

fn remaining(resubmission: Resubmission) -> u16 {
    match resubmission {
        Resubmission::Makeup(offer) => offer.balance.remaining(),
        other => panic!("expected a makeup, got {other:?}"),
    }
}

#[test]
fn before_the_deadline_a_resubmission_is_regular_and_on_time() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let work = submission(&assignment, &student).build();
    let at = assignment.deadline_at() - Duration::minutes(1);

    assert_eq!(
        work.resubmission_at(&assignment, pending(), None, at),
        Resubmission::Regular {
            closes_at: assignment.deadline_at(),
            late: false,
        }
    );
}

#[test]
fn after_the_deadline_within_the_late_policy_a_resubmission_is_late() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let work = submission(&assignment, &student).build();

    assert_eq!(
        work.resubmission_at(
            &assignment,
            pending(),
            None,
            deadline_at + Duration::days(1)
        ),
        Resubmission::Regular {
            closes_at: assignment.closes_at(),
            late: true,
        }
    );
}

#[test]
fn after_closing_a_regular_submission_is_resubmitted_only_as_a_makeup() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = credit_assignment(&course, &teacher);
    let work = submission(&assignment, &student).build();
    let window = makeup_window(3, t0());
    let pool = MakeupPool {
        course: window,
        balance: window.balance(0, 1),
    };

    let resubmission = work.resubmission_at(
        &assignment,
        pending(),
        Some(pool),
        after_closing(&assignment),
    );

    assert_eq!(remaining(resubmission), 2);
}

#[test]
fn after_closing_without_a_makeup_a_resubmission_is_refused() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).published().build();
    let work = submission(&assignment, &student).build();
    let at = after_closing(&assignment);

    assert_eq!(
        work.resubmission_at(&assignment, pending(), None, at),
        Resubmission::Refused(DomainError::AssignmentDeadlineExceeded {
            deadline_at: assignment.deadline_at(),
            submitted_at: at,
        })
    );
}

#[test]
fn a_withdrawn_makeup_returns_its_attempt_to_the_pool() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = credit_assignment(&course, &teacher);
    let (makeup, pool) = pending_makeup(&assignment, &student, makeup_window(3, t0()), 0);
    let at = makeup.submitted_at() + Duration::hours(1);

    assert_eq!(
        remaining(makeup.resubmission_at(&assignment, pending(), Some(pool), at)),
        3
    );
}

#[test]
fn withdrawing_the_makeup_of_the_last_attempt_leaves_one_attempt() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = credit_assignment(&course, &teacher);
    let (makeup, pool) = pending_makeup(&assignment, &student, makeup_window(1, t0()), 0);
    assert_eq!(pool.balance.remaining(), 0);
    let at = makeup.submitted_at() + Duration::hours(1);

    assert_eq!(
        remaining(makeup.resubmission_at(&assignment, pending(), Some(pool), at)),
        1
    );
}

#[test]
fn a_makeup_of_an_earlier_enabling_returns_nothing_to_the_new_pool() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = credit_assignment(&course, &teacher);
    let (makeup, _) = pending_makeup(&assignment, &student, makeup_window(3, t0()), 0);
    let reopened = makeup_window(3, makeup.submitted_at() + Duration::hours(1));
    let pool = MakeupPool {
        course: reopened,
        balance: reopened.balance(0, 1),
    };
    let at = reopened.opened_at() + Duration::hours(1);

    assert_eq!(
        remaining(makeup.resubmission_at(&assignment, pending(), Some(pool), at)),
        2
    );
}

#[test]
fn an_already_withdrawn_makeup_returns_nothing_again() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = credit_assignment(&course, &teacher);
    let window = makeup_window(3, t0());
    let (mut makeup, _) = pending_makeup(&assignment, &student, window, 1);
    let at = makeup.submitted_at() + Duration::hours(1);
    makeup.withdraw(&student, at).unwrap();
    let pool = MakeupPool {
        course: window,
        balance: window.balance(0, 1),
    };

    assert_eq!(
        remaining(makeup.resubmission_at(&assignment, history().build(), Some(pool), at)),
        2
    );
}
