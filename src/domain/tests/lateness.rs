use chrono::{DateTime, Duration, TimeZone, Utc};

use super::{Fixture, fixture, out_of, reviewed_at, weekly_penalty_assignment};
use crate::domain::{
    Assignment, DeductedFrom, DomainError, LatePenalty, LatePolicy, SubmissionStatus,
};
use crate::test_support::{ReviewShortcuts, assignment, student, submission, t0};

fn points_at(assignment: &Assignment, at: DateTime<Utc>) -> Option<u16> {
    assignment.late_penalty_at(at).map(LatePenalty::points)
}

#[test]
fn the_deadline_moment_itself_is_already_late() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    let just_before = deadline_at - Duration::nanoseconds(1);
    assert_eq!(assignment.late_periods_at(just_before), 0);
    assert_eq!(assignment.late_penalty_at(just_before), None);
    assert_eq!(assignment.late_periods_at(deadline_at), 1);
    assert_eq!(points_at(&assignment, deadline_at), Some(1));
}

#[test]
fn any_lateness_costs_the_first_period_at_once() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let at = Utc.with_ymd_and_hms(2026, 8, 25, 0, 0, 0).unwrap();

    assert_eq!(assignment.late_periods_at(at), 1);
    assert_eq!(points_at(&assignment, at), Some(1));
}

#[test]
fn zero_possible_score_does_not_close_submissions_early() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = assignment(&course, &teacher)
        .scored(1)
        .late_policy(7, 1, 4, DeductedFrom::WorkScore)
        .published()
        .build();
    let last_moment = work.closes_at() - Duration::nanoseconds(1);
    assert_eq!(points_at(&work, last_moment), Some(4));
    assert!(
        submission(&work, &student)
            .submitted_at(last_moment)
            .try_build()
            .is_ok()
    );
}

#[test]
fn the_second_period_starts_exactly_one_week_after_the_first() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let second_period = Utc.with_ymd_and_hms(2026, 8, 31, 23, 59, 0).unwrap();

    assert_eq!(
        assignment.late_periods_at(second_period - Duration::nanoseconds(1)),
        1
    );
    assert_eq!(assignment.late_periods_at(second_period), 2);
}

#[test]
fn each_period_boundary_belongs_to_the_next_period_down_to_a_fraction_of_a_second() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    for period in 0..4 {
        let boundary = deadline_at + Duration::weeks(period);
        assert_eq!(assignment.late_periods_at(boundary), period as u32 + 1);
        for fraction in [Duration::nanoseconds(1), Duration::milliseconds(500)] {
            assert_eq!(
                assignment.late_periods_at(boundary - fraction),
                period as u32
            );
        }
    }
}

#[test]
fn the_horizon_is_closed_at_its_own_moment_and_open_just_before_it() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let closes_at = Utc.with_ymd_and_hms(2026, 9, 21, 23, 59, 0).unwrap();
    assert_eq!(assignment.closes_at(), closes_at);

    let last_moment = closes_at - Duration::nanoseconds(1);
    assignment.ensure_open_for_submission(last_moment).unwrap();
    assert_eq!(assignment.late_periods_at(last_moment), 4);
    assert_eq!(
        assignment
            .ensure_open_for_submission(closes_at)
            .unwrap_err(),
        DomainError::SubmissionWindowClosed {
            closes_at,
            submitted_at: closes_at,
        }
    );
}

#[test]
fn a_late_submission_before_the_horizon_is_accepted() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    let submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(1))
        .try_build();

    assert!(submission.is_ok());
}

#[test]
fn a_review_freezes_the_lateness_of_the_submission_not_of_the_review() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(1))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(5, 5),
            "Ок",
            deadline_at + Duration::days(30),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.max_score(), 5);
    assert_eq!(
        grade.late_periods(),
        1,
        "проверка через месяц ничего не меняет"
    );
    assert_eq!(grade.penalty(), 1);
    assert_eq!(grade.work_score(), 4);
}

#[test]
fn a_penalty_larger_than_the_score_leaves_an_accepted_submission_with_zero() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(28) - Duration::nanoseconds(1))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(4, 5),
            "Ок",
            deadline_at + Duration::days(29),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.raw_score(), 4);
    assert_eq!(grade.max_score(), 5);
    assert_eq!(grade.penalty(), 4);
    assert_eq!(grade.work_score(), 0);
    assert_eq!(submission.status(), SubmissionStatus::Accepted);
}

#[test]
fn an_effective_penalty_is_capped_at_the_global_score_limit() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let deadline_at = t0() + Duration::days(7);
    let assignment = assignment(&course, &teacher)
        .deadline_at(deadline_at)
        .scored(100)
        .late_policy(1, 100, 2, DeductedFrom::WorkScore)
        .published()
        .build();
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(2) - Duration::nanoseconds(1))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(100, 100),
            "Ок",
            deadline_at + Duration::days(3),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.max_score(), 100);
    assert_eq!(grade.late_periods(), 2);
    assert_eq!(grade.penalty(), 100);
    assert_eq!(grade.work_score(), 0);
    assert_eq!(submission.status(), SubmissionStatus::Accepted);
}

#[test]
fn a_deduction_change_after_a_review_applies_only_to_later_reviews() {
    let Fixture {
        teacher,
        group,
        course,
        ..
    } = fixture();
    let (mut assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let submitted_at = deadline_at + Duration::days(1);
    let mut first = submission(&assignment, &student(2, &group))
        .submitted_at(submitted_at)
        .build();
    let mut second = submission(&assignment, &student(3, &group))
        .submitted_at(submitted_at)
        .build();
    first
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(5, 5),
            "Ок",
            reviewed_at(),
        )
        .unwrap();
    let given = first.grade().unwrap();

    assignment
        .change_grading(
            &course,
            &teacher,
            assignment.grading(),
            Some(LatePolicy::new(7, 1, 4, DeductedFrom::CourseTotal).unwrap()),
        )
        .unwrap();
    second
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(5, 5),
            "Ок",
            reviewed_at(),
        )
        .unwrap();

    assert_eq!(first.grade(), Some(given));
    assert_eq!(given.deducted_from(), Some(DeductedFrom::WorkScore));
    assert_eq!((given.work_score(), given.course_total_penalty()), (4, 0));
    let later = second.grade().unwrap();
    assert_eq!(later.deducted_from(), Some(DeductedFrom::CourseTotal));
    assert_eq!((later.work_score(), later.course_total_penalty()), (5, 1));
}

#[test]
fn an_on_time_submission_carries_no_deduction_whatever_the_policy() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at - Duration::nanoseconds(1))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(5, 5),
            "Ок",
            reviewed_at(),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.late(), None);
    assert_eq!((grade.work_score(), grade.course_total_penalty()), (5, 0));
}

#[test]
fn nonweekly_closing_is_an_exact_exclusive_cutoff() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let deadline = t0() + Duration::days(7);
    let closing = deadline + Duration::days(10);
    let work = assignment(&course, &teacher)
        .scored(5)
        .deadline_at(deadline)
        .late_policy(5, 1, 2, DeductedFrom::WorkScore)
        .build();
    assert_eq!(work.closes_at(), closing);
    assert_eq!(
        points_at(&work, closing - Duration::nanoseconds(1)),
        Some(2)
    );
    assert!(
        work.ensure_open_for_submission(closing - Duration::nanoseconds(1))
            .is_ok()
    );
    assert!(matches!(
        work.ensure_open_for_submission(closing),
        Err(DomainError::SubmissionWindowClosed { .. })
    ));
}
