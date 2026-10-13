use chrono::{DateTime, Duration, TimeZone, Utc};

use super::{Fixture, fixture, weekly_penalty_assignment};
use crate::domain::{DeductedFrom, DomainError, Grading, LATEST_TERM_YEAR, LatePolicy};
use crate::test_support::{assignment, t0};

#[test]
fn creating_an_assignment_rejects_a_horizon_longer_than_366_days() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    assert_eq!(
        assignment(&course, &teacher)
            .scored(5)
            .late_policy(365, 1, 100, DeductedFrom::WorkScore)
            .try_build(),
        Err(DomainError::LateHorizonTooLong { max_days: 366 })
    );
}

#[test]
fn a_failed_deadline_change_preserves_the_previous_horizon() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut work = assignment(&course, &teacher)
        .scored(5)
        .late_policy(1, 1, 2, DeductedFrom::WorkScore)
        .build();
    let before = work.clone();
    assert_eq!(
        work.change_deadline(
            &course,
            &teacher,
            DateTime::<Utc>::MAX_UTC - Duration::days(1)
        ),
        Err(DomainError::AssignmentClosingOutOfRange)
    );
    assert_eq!(work, before);
}

#[test]
fn assignment_terms_end_in_the_last_supported_year() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let deadline_at = |year| Utc.with_ymd_and_hms(year, 12, 31, 0, 0, 0).unwrap();

    assert!(
        assignment(&course, &teacher)
            .deadline_at(deadline_at(LATEST_TERM_YEAR))
            .try_build()
            .is_ok()
    );
    assert_eq!(
        assignment(&course, &teacher)
            .deadline_at(deadline_at(LATEST_TERM_YEAR + 1))
            .try_build(),
        Err(DomainError::AssignmentClosingOutOfRange)
    );
}

#[test]
fn adding_a_policy_rejects_a_closing_out_of_range_without_changing_the_scale() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut work = assignment(&course, &teacher)
        .deadline_at(
            Utc.with_ymd_and_hms(LATEST_TERM_YEAR, 12, 31, 0, 0, 0)
                .unwrap(),
        )
        .build();
    let before = work.clone();
    assert_eq!(
        work.change_grading(
            &course,
            &teacher,
            Grading::scored(5).unwrap(),
            Some(LatePolicy::new(1, 1, 1, DeductedFrom::WorkScore).unwrap())
        ),
        Err(DomainError::AssignmentClosingOutOfRange)
    );
    assert_eq!(work, before);
}

#[test]
fn exactly_366_days_of_lateness_are_allowed() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = assignment(&course, &teacher)
        .scored(10)
        .late_policy(183, 1, 2, DeductedFrom::WorkScore)
        .build();
    assert_eq!(work.closes_at(), work.deadline_at() + Duration::days(366));
}

#[test]
fn the_shortest_representable_horizon_over_366_days_is_rejected() {
    let Fixture {
        teacher, course, ..
    } = fixture();

    assert_eq!(
        assignment(&course, &teacher)
            .scored(10)
            .late_policy(184, 1, 2, DeductedFrom::WorkScore)
            .try_build(),
        Err(DomainError::LateHorizonTooLong { max_days: 366 })
    );
}

#[test]
fn changing_only_the_penalty_preserves_the_horizon() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (mut work, _) = weekly_penalty_assignment(&course, &teacher);
    let closing = work.closes_at();
    work.change_grading(
        &course,
        &teacher,
        work.grading(),
        Some(LatePolicy::new(7, 2, 4, DeductedFrom::WorkScore).unwrap()),
    )
    .unwrap();
    assert_eq!(work.closes_at(), closing);
}

#[test]
fn changing_the_period_recalculates_the_horizon() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (mut work, deadline) = weekly_penalty_assignment(&course, &teacher);
    work.change_terms(
        &course,
        &teacher,
        deadline,
        Some(LatePolicy::new(10, 1, 4, DeductedFrom::WorkScore).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(work.closes_at(), deadline + Duration::days(40));
}

#[test]
fn creation_rejects_an_unrepresentable_closing() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    assert_eq!(
        assignment(&course, &teacher)
            .scored(10)
            .deadline_at(DateTime::<Utc>::MAX_UTC)
            .late_policy(1, 1, 1, DeductedFrom::WorkScore)
            .try_build(),
        Err(DomainError::AssignmentClosingOutOfRange)
    );
}

#[test]
fn moving_the_deadline_shifts_the_closing_date() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (mut assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let closes_at = assignment.closes_at();

    assignment
        .change_deadline(&course, &teacher, deadline_at + Duration::days(7))
        .unwrap();

    assert_eq!(assignment.closes_at(), closes_at + Duration::days(7));
}

#[test]
fn a_late_policy_needs_a_scored_scale() {
    let Fixture {
        teacher, course, ..
    } = fixture();

    let result = assignment(&course, &teacher)
        .late_policy(7, 1, 4, DeductedFrom::WorkScore)
        .try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::LatePolicyRequiresScoredGrading
    );
}

#[test]
fn deadline_and_policy_can_change_atomically() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (mut work, _) = weekly_penalty_assignment(&course, &teacher);
    let deadline = work.closes_at() + Duration::days(1);
    let closing = deadline + Duration::days(10);
    work.change_terms(
        &course,
        &teacher,
        deadline,
        Some(LatePolicy::new(5, 1, 2, DeductedFrom::WorkScore).unwrap()),
        None,
    )
    .unwrap();
    assert_eq!(work.deadline_at(), deadline);
    assert_eq!(work.closes_at(), closing);
    let before = work.clone();
    assert_eq!(
        work.change_terms(
            &course,
            &teacher,
            deadline + Duration::days(1),
            Some(LatePolicy::new(365, 1, 2, DeductedFrom::WorkScore).unwrap()),
            None,
        ),
        Err(DomainError::LateHorizonTooLong { max_days: 366 })
    );
    assert_eq!(work, before);
}

#[test]
fn pass_fail_assignments_cannot_gain_a_policy_through_terms_changes() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut work = assignment(&course, &teacher).build();
    let before = work.clone();
    assert_eq!(
        work.change_terms(
            &course,
            &teacher,
            t0() + Duration::days(8),
            Some(LatePolicy::new(1, 1, 2, DeductedFrom::WorkScore).unwrap()),
            None,
        ),
        Err(DomainError::LatePolicyRequiresScoredGrading)
    );
    assert_eq!(work, before);
}

#[test]
fn dropping_the_scale_of_an_assignment_with_a_late_policy_is_rejected() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let (mut assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let result = assignment.change_grading(
        &course,
        &teacher,
        Grading::PassFail,
        assignment.late_policy(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::LatePolicyRequiresScoredGrading
    );
}
