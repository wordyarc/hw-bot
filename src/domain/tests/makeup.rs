use chrono::{DateTime, Duration, Utc};

use super::{Fixture, fixture, reviewed_at};
use crate::domain::{
    Acceptance, Assignment, CourseMakeup, DeductedFrom, DomainError, DomainResult, Grade, Grading,
    LatePolicy, MAX_ATTEMPTS, MakeupOffer, MakeupPool, MakeupRequest, MakeupTerms,
    SubmissionHistory, SubmissionKind, SubmissionStart, SubmissionStatus, TeachingCourse, User,
};
use crate::test_support::{ReviewShortcuts, assignment, history, out_of, submission, t0, teacher};

fn makeup_window() -> CourseMakeup {
    CourseMakeup::open(t0() + Duration::days(60), 3, t0()).unwrap()
}

fn makeup_with_used(used: u16) -> MakeupRequest {
    let window = makeup_window();
    MakeupRequest {
        course: Some(window),
        consent: true,
        balance: window.balance(0, used),
    }
}

fn credit_assignment(course: &TeachingCourse, teacher: &User) -> Assignment {
    assignment(course, teacher)
        .max_attempts(2)
        .scored(10)
        .makeup_credit(6)
        .published()
        .build()
}

fn after_closing(assignment: &Assignment) -> DateTime<Utc> {
    assignment.closes_at() + Duration::days(1)
}

fn closed_at(assignment: &Assignment, at: DateTime<Utc>) -> DomainError {
    DomainError::AssignmentDeadlineExceeded {
        deadline_at: assignment.deadline_at(),
        submitted_at: at,
    }
}

#[test]
fn makeup_terms_must_fit_the_scale_while_not_offered_fits_any() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut credited = credit_assignment(&course, &teacher);
    let before = credited.clone();

    for grading in [Grading::scored(5).unwrap(), Grading::PassFail] {
        assert_eq!(
            credited.change_grading(&course, &teacher, grading, None),
            Err(DomainError::MakeupTermsDoNotFitGrading),
            "{grading:?}"
        );
        assert_eq!(credited, before);
    }
    let mut declined = assignment(&course, &teacher)
        .scored(10)
        .makeup(MakeupTerms::NotOffered)
        .build();
    declined
        .change_grading(&course, &teacher, Grading::scored(5).unwrap(), None)
        .unwrap();
    assert_eq!(declined.makeup(), Some(MakeupTerms::NotOffered));
    assert_eq!(
        assignment(&course, &teacher)
            .scored(5)
            .makeup(MakeupTerms::scored(6, Grading::scored(10).unwrap()).unwrap())
            .try_build(),
        Err(DomainError::MakeupTermsDoNotFitGrading)
    );
    assert_eq!(
        assignment(&course, &teacher)
            .makeup(MakeupTerms::PassFail)
            .try_build()
            .unwrap()
            .makeup(),
        Some(MakeupTerms::PassFail)
    );
}

#[test]
fn changing_terms_carries_the_makeup_terms_and_checks_them_against_the_scale() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut work = credit_assignment(&course, &teacher);
    let deadline = work.deadline_at() + Duration::days(1);

    work.change_terms(
        &course,
        &teacher,
        deadline,
        None,
        Some(MakeupTerms::NotOffered),
    )
    .unwrap();
    assert_eq!(work.makeup(), Some(MakeupTerms::NotOffered));
    assert_eq!(
        work.change_terms(
            &course,
            &teacher,
            deadline,
            None,
            Some(MakeupTerms::PassFail)
        ),
        Err(DomainError::MakeupTermsDoNotFitGrading)
    );
    work.change_deadline(&course, &teacher, deadline + Duration::days(1))
        .unwrap();
    assert_eq!(work.makeup(), Some(MakeupTerms::NotOffered));
}

#[test]
fn deciding_makeup_terms_is_refused_for_an_archived_assignment() {
    let outsider = teacher(2);
    let Fixture {
        teacher, course, ..
    } = fixture();
    let mut work = assignment(&course, &teacher).scored(10).build();
    let six = MakeupTerms::scored(6, work.grading()).unwrap();

    work.decide_makeup(&course, &teacher, six).unwrap();
    assert_eq!(work.makeup(), Some(six));
    assert_eq!(
        work.decide_makeup(&course, &outsider, MakeupTerms::NotOffered),
        Err(DomainError::PermissionDenied {
            action: "decide assignment makeup"
        })
    );
    work.archive(&course, &teacher).unwrap();
    assert_eq!(
        work.decide_makeup(&course, &teacher, MakeupTerms::NotOffered),
        Err(DomainError::AssignmentIsArchived)
    );
}

#[test]
fn the_makeup_window_opens_at_the_closing_and_ends_just_before_its_end() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let window = makeup_window();
    let closes_at = work.closes_at();
    let makeup = Acceptance::Makeup {
        credit: work.makeup().and_then(MakeupTerms::credit),
    };

    for (at, expected) in [
        (
            closes_at - Duration::nanoseconds(1),
            Ok(Acceptance::Regular),
        ),
        (closes_at, Ok(makeup)),
        (window.until() - Duration::nanoseconds(1), Ok(makeup)),
        (window.until(), Err(closed_at(&work, window.until()))),
    ] {
        assert_eq!(work.acceptance_at(at, Some(window)), expected, "{at}");
    }
}

#[test]
fn the_makeup_window_opens_no_earlier_than_the_last_opening() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let opened_at = work.closes_at() + Duration::days(3);
    let window = CourseMakeup::open(opened_at + Duration::days(30), 3, opened_at).unwrap();

    let before = opened_at - Duration::nanoseconds(1);
    assert_eq!(
        work.acceptance_at(before, Some(window)),
        Err(closed_at(&work, before))
    );
    assert_eq!(
        work.acceptance_at(opened_at, Some(window))
            .map(Acceptance::kind),
        Ok(SubmissionKind::Makeup)
    );
}

#[test]
fn an_assignment_closing_after_the_end_of_makeup_has_no_makeup() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let window = makeup_window();
    let work = assignment(&course, &teacher)
        .scored(10)
        .makeup_credit(6)
        .deadline_at(window.until() + Duration::days(1))
        .published()
        .build();

    let at = after_closing(&work);
    assert_eq!(
        work.acceptance_at(at, Some(window)),
        Err(closed_at(&work, at))
    );
}

#[test]
fn undecided_or_declined_makeup_keeps_the_closing_code() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let window = makeup_window();
    let strict = assignment(&course, &teacher).scored(10).published().build();
    let late = assignment(&course, &teacher)
        .scored(10)
        .late_policy(7, 1, 2, DeductedFrom::WorkScore)
        .makeup(MakeupTerms::NotOffered)
        .published()
        .build();

    let at = after_closing(&strict);
    assert_eq!(
        strict.acceptance_at(at, Some(window)),
        Err(closed_at(&strict, at))
    );
    let at = after_closing(&late);
    assert_eq!(
        late.acceptance_at(at, Some(window)),
        Err(DomainError::SubmissionWindowClosed {
            closes_at: late.closes_at(),
            submitted_at: at,
        })
    );
    let credited = credit_assignment(&course, &teacher);
    let at = after_closing(&credited);
    assert_eq!(
        credited.acceptance_at(at, None),
        Err(closed_at(&credited, at))
    );
}

#[test]
fn an_accepted_work_closes_the_makeup_only_at_the_credit_or_without_a_score() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let pass_fail = assignment(&course, &teacher)
        .makeup(MakeupTerms::PassFail)
        .published()
        .build();
    let to_total = Grade::new(
        8,
        10,
        LatePolicy::new(7, 4, 4, DeductedFrom::CourseTotal)
            .unwrap()
            .late_penalty(1),
    );
    let already_accepted = Err(DomainError::SubmissionAlreadyAccepted);

    for (assignment, history, expected) in [
        (&work, history().used(1).accepted_with(5), Ok(())),
        (
            &work,
            history().used(1).accepted_with(6),
            already_accepted.clone(),
        ),
        (
            &work,
            history().used(1).accepted_with(to_total.work_score()),
            already_accepted.clone(),
        ),
        (
            &work,
            history().used(1).accepted(),
            already_accepted.clone(),
        ),
        (
            &pass_fail,
            history().used(1).accepted(),
            already_accepted.clone(),
        ),
        (
            &work,
            history().used(1).accepted_makeup().submissions(2),
            already_accepted.clone(),
        ),
        (
            &work,
            history().used(1).pending().submissions(2),
            Err(DomainError::SubmissionAlreadyPendingReview),
        ),
    ] {
        let result = submission(assignment, &student)
            .history(history.build())
            .submitted_at(after_closing(assignment))
            .makeup(makeup_with_used(0))
            .try_build()
            .map(|submission| assert_eq!(submission.kind(), SubmissionKind::Makeup));
        assert_eq!(result, expected, "{history:?}");
    }
}

#[test]
fn an_empty_pool_is_named_only_when_nothing_else_blocks_the_makeup() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    for (history, expected) in [
        (
            history().used(1).pending().submissions(2),
            DomainError::SubmissionAlreadyPendingReview,
        ),
        (
            history().used(1).accepted_with(6),
            DomainError::SubmissionAlreadyAccepted,
        ),
        (
            history().used(1).accepted_with(5),
            DomainError::MakeupAttemptsExhausted {
                allowed: 3,
                used: 3,
            },
        ),
    ] {
        assert_eq!(
            submission(&work, &student)
                .history(history.build())
                .submitted_at(after_closing(&work))
                .makeup(makeup_with_used(3))
                .try_build(),
            Err(expected),
            "{history:?}"
        );
    }
}

#[test]
fn a_makeup_without_consent_keeps_the_closing_code_and_consent_does_not_block_a_regular_one() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let at = after_closing(&work);

    let without_consent = MakeupRequest {
        consent: false,
        ..makeup_with_used(0)
    };
    assert_eq!(
        submission(&work, &student)
            .submitted_at(at)
            .makeup(without_consent)
            .try_build(),
        Err(closed_at(&work, at))
    );
    let regular = submission(&work, &student)
        .submitted_at(work.closes_at() - Duration::nanoseconds(1))
        .makeup(makeup_with_used(3))
        .build();
    assert_eq!(regular.kind(), SubmissionKind::Regular);
}

#[test]
fn a_credited_makeup_closes_regular_submissions_after_the_deadline_moves() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    assert_eq!(
        submission(&work, &student)
            .history(history().used(1).accepted_makeup().submissions(2).build())
            .makeup(makeup_with_used(0))
            .try_build(),
        Err(DomainError::SubmissionAlreadyAccepted)
    );
}

#[test]
fn a_makeup_ignores_exhausted_attempts_and_continues_the_numbering() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let rejected_twice = history().used(2).build();

    assert_eq!(
        submission(&work, &student)
            .history(rejected_twice)
            .try_build(),
        Err(DomainError::AttemptsExhausted {
            allowed: 2,
            used: 2
        })
    );
    let makeup = submission(&work, &student)
        .history(rejected_twice)
        .submitted_at(after_closing(&work))
        .makeup(makeup_with_used(0))
        .build();
    assert_eq!(makeup.kind(), SubmissionKind::Makeup);
    assert_eq!(makeup.attempt_number(), 3);

    let after_rejected_makeup = history().used(2).submissions(3).build();
    assert_eq!(after_rejected_makeup.used_attempts(), 2);
    assert_eq!(after_rejected_makeup.remaining_attempts_for(&work), 0);
    let next = submission(&work, &student)
        .history(after_rejected_makeup)
        .submitted_at(after_closing(&work))
        .makeup(makeup_with_used(1))
        .build();
    assert_eq!(next.attempt_number(), 4);
}

#[test]
fn the_attempt_number_never_leaves_the_storage_range() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    assert_eq!(
        submission(&work, &student)
            .history(history().submissions(MAX_ATTEMPTS).build())
            .submitted_at(after_closing(&work))
            .makeup(makeup_with_used(0))
            .try_build(),
        Err(DomainError::AttemptLimitExceeded {
            value: u32::from(MAX_ATTEMPTS) + 1,
            max: MAX_ATTEMPTS,
        })
    );
}

#[test]
fn the_card_offers_a_makeup_with_its_balance_or_nothing() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let at = after_closing(&work);
    let window = makeup_window();
    let offer = |history: SubmissionHistory, used| {
        history.makeup_offer_at(&work, at, window, window.balance(0, used))
    };

    assert_eq!(
        offer(history().used(1).accepted_with(5).build(), 1),
        Some(MakeupOffer {
            until: window.until(),
            credit: work.makeup().and_then(MakeupTerms::credit),
            balance: window.balance(0, 1),
        })
    );
    let Some(MakeupOffer { balance, .. }) = offer(history().build(), 3) else {
        panic!("an empty pool still shows the makeup");
    };
    assert_eq!(balance.remaining(), 0);
    assert_eq!(offer(history().used(1).accepted_with(6).build(), 0), None);
    assert_eq!(
        history().build().makeup_offer_at(
            &work,
            work.deadline_at() - Duration::hours(1),
            window,
            window.balance(0, 0)
        ),
        None
    );
}

fn pool(used: u16) -> MakeupPool {
    let course = makeup_window();
    MakeupPool {
        course,
        balance: course.balance(0, used),
    }
}

fn reopened_pool(reopened_at: DateTime<Utc>, used: u16) -> MakeupPool {
    let course = CourseMakeup::open(makeup_window().until(), 3, reopened_at).unwrap();
    MakeupPool {
        course,
        balance: course.balance(0, used),
    }
}

fn consented(start: DomainResult<SubmissionStart>) -> Option<bool> {
    match start {
        Ok(SubmissionStart::Makeup { consented, .. }) => Some(consented),
        _ => None,
    }
}

#[test]
fn a_makeup_button_shown_before_the_end_admits_work_handled_after_it() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let until = makeup_window().until();

    let start = history().build().submission_start(
        &work,
        Some(pool(0)),
        SubmissionKind::Makeup,
        until - Duration::hours(1),
        until + Duration::hours(1),
    );

    assert_eq!(consented(start), Some(true));
}

#[test]
fn a_button_shown_after_the_makeup_end_is_refused_as_closed() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = makeup_window().until();

    assert_eq!(
        history().build().submission_start(
            &work,
            Some(pool(0)),
            SubmissionKind::Makeup,
            shown_at,
            shown_at + Duration::hours(1)
        ),
        Err(closed_at(&work, shown_at))
    );
}

#[test]
fn a_regular_button_handled_after_closing_asks_for_makeup_consent_in_the_prompt() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    let start = history().build().submission_start(
        &work,
        Some(pool(0)),
        SubmissionKind::Regular,
        work.closes_at() - Duration::hours(1),
        after_closing(&work),
    );

    assert_eq!(consented(start), Some(false));
}

#[test]
fn a_regular_button_shown_in_the_makeup_window_does_not_consent_to_a_makeup() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = after_closing(&work);

    let start = history().build().submission_start(
        &work,
        Some(pool(0)),
        SubmissionKind::Regular,
        shown_at,
        shown_at + Duration::hours(1),
    );

    assert_eq!(consented(start), Some(false));
}

#[test]
fn a_regular_button_shown_in_the_makeup_window_is_refused_after_its_end() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = after_closing(&work);

    assert_eq!(
        history().build().submission_start(
            &work,
            Some(pool(0)),
            SubmissionKind::Regular,
            shown_at,
            makeup_window().until()
        ),
        Err(closed_at(&work, shown_at))
    );
}

#[test]
fn a_regular_button_without_a_makeup_stays_regular_whenever_it_is_handled() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = work.closes_at() - Duration::hours(1);

    for now in [shown_at, after_closing(&work)] {
        assert_eq!(
            history()
                .build()
                .submission_start(&work, None, SubmissionKind::Regular, shown_at, now),
            Ok(SubmissionStart::Regular)
        );
    }
}

#[test]
fn an_empty_pool_names_the_pool_instead_of_the_closed_deadline() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = after_closing(&work);

    assert_eq!(
        history().build().submission_start(
            &work,
            Some(pool(3)),
            SubmissionKind::Makeup,
            shown_at,
            shown_at
        ),
        Err(DomainError::MakeupAttemptsExhausted {
            allowed: 3,
            used: 3
        })
    );
}

#[test]
fn a_button_shown_before_the_makeup_was_reopened_asks_for_consent_in_the_prompt() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = after_closing(&work);
    let reopened_at = shown_at + Duration::days(1);

    let start = history().build().submission_start(
        &work,
        Some(reopened_pool(reopened_at, 0)),
        SubmissionKind::Makeup,
        shown_at,
        reopened_at + Duration::hours(1),
    );

    assert_eq!(consented(start), Some(false));
}

#[test]
fn a_button_shown_before_the_makeup_was_reopened_names_the_spent_new_pool() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);
    let shown_at = after_closing(&work);
    let reopened_at = shown_at + Duration::days(1);

    assert_eq!(
        history().build().submission_start(
            &work,
            Some(reopened_pool(reopened_at, 3)),
            SubmissionKind::Makeup,
            shown_at,
            reopened_at + Duration::hours(1)
        ),
        Err(DomainError::MakeupAttemptsExhausted {
            allowed: 3,
            used: 3
        })
    );
}

#[test]
fn a_regular_button_with_spent_attempts_asks_for_makeup_consent_after_closing() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    let start = history().used(2).submissions(2).build().submission_start(
        &work,
        Some(pool(0)),
        SubmissionKind::Regular,
        work.closes_at() - Duration::hours(1),
        after_closing(&work),
    );

    assert_eq!(consented(start), Some(false));
}

#[test]
fn a_regular_button_with_spent_attempts_names_them_without_a_makeup() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let work = credit_assignment(&course, &teacher);

    assert_eq!(
        history().used(2).submissions(2).build().submission_start(
            &work,
            None,
            SubmissionKind::Regular,
            work.closes_at() - Duration::hours(1),
            after_closing(&work)
        ),
        Err(DomainError::AttemptsExhausted {
            allowed: 2,
            used: 2
        })
    );
}

#[test]
fn a_makeup_is_credited_by_the_terms_at_review_without_a_penalty() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let mut work = assignment(&course, &teacher)
        .scored(10)
        .late_policy(7, 3, 2, DeductedFrom::CourseTotal)
        .makeup_credit(6)
        .published()
        .build();
    let mut makeup = submission(&work, &student)
        .submitted_at(after_closing(&work) + Duration::days(30))
        .makeup(makeup_with_used(0))
        .build();

    assert_eq!(
        makeup.accept(
            &work,
            &course,
            &teacher,
            Some(out_of(6, 10)),
            "Ок",
            reviewed_at()
        ),
        Err(DomainError::ScoreNotAllowed)
    );
    assert!(makeup.is_pending_review());
    work.change_terms(
        &course,
        &teacher,
        work.deadline_at(),
        work.late_policy(),
        Some(MakeupTerms::scored(5, work.grading()).unwrap()),
    )
    .unwrap();
    makeup
        .accept(&work, &course, &teacher, None, "Зачтено", reviewed_at())
        .unwrap();

    let grade = makeup.grade().unwrap();
    assert_eq!((grade.raw_score(), grade.max_score()), (5, 10));
    assert_eq!(grade.late(), None);
    assert_eq!((grade.work_score(), grade.course_total_penalty()), (5, 0));
}

#[test]
fn a_makeup_with_withdrawn_terms_can_be_rejected_but_not_accepted() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let mut work = credit_assignment(&course, &teacher);
    let mut makeup = submission(&work, &student)
        .submitted_at(after_closing(&work))
        .makeup(makeup_with_used(0))
        .build();
    work.change_terms(
        &course,
        &teacher,
        work.deadline_at(),
        None,
        Some(MakeupTerms::NotOffered),
    )
    .unwrap();

    assert_eq!(
        makeup.accept(&work, &course, &teacher, None, "Ок", reviewed_at()),
        Err(DomainError::MakeupNotOffered)
    );
    assert!(makeup.is_pending_review());
    makeup
        .reject(&work, &course, &teacher, "Условия сняты", reviewed_at())
        .unwrap();
    assert_eq!(makeup.status(), SubmissionStatus::Rejected);
}

#[test]
fn a_pass_fail_makeup_is_accepted_without_a_grade() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let work = assignment(&course, &teacher)
        .makeup(MakeupTerms::PassFail)
        .published()
        .build();
    let mut makeup = submission(&work, &student)
        .submitted_at(after_closing(&work))
        .makeup(makeup_with_used(0))
        .build();

    makeup
        .accept(&work, &course, &teacher, None, "Зачтено", reviewed_at())
        .unwrap();

    assert_eq!(makeup.status(), SubmissionStatus::Accepted);
    assert_eq!(makeup.grade(), None);
}
