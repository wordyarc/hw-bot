use std::collections::HashSet;

use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::domain::{
    Acceptance, Assignment, AssignmentId, AssignmentStatus, CourseMakeup, DeductedFrom,
    DomainError, Grade, Grading, LATEST_TERM_YEAR, LatePenalty, LatePolicy, MakeupOffer,
    MakeupRequest, MakeupTerms, NewAssignment, Review, ReviewDecision, ReviewScore, Role,
    StaffRole, StudentAssignmentProgress, Submission, SubmissionHistory, SubmissionId,
    SubmissionKind, SubmissionStatus, TeachingCourse, TelegramUserId, USER_TEXT_MAX_LEN, User,
    UserId,
};
use crate::test_support::{
    ASSIGNMENT_BODY, ASSIGNMENT_TITLE, GROUP_CODE, OTHER_GROUP_CODE, PULL_REQUEST_URL,
    ReviewShortcuts, SUBJECT_NAME, admin, assignment, course_for, history, student, student_group,
    subject, submission, t0, teacher, teaching_course,
};

fn reviewed_at() -> DateTime<Utc> {
    t0() + Duration::hours(2)
}

fn points_at(assignment: &Assignment, at: DateTime<Utc>) -> Option<u16> {
    assignment.late_penalty_at(at).map(LatePenalty::points)
}

fn out_of(value: u16, max_score: u16) -> Option<ReviewScore> {
    Some(ReviewScore { value, max_score })
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
        NewAssignment {
            title: ASSIGNMENT_TITLE.to_owned(),
            body: ASSIGNMENT_BODY.to_owned(),
            groups: None,
            max_attempts: 2,
            grading: Grading::PassFail,
            late_policy: None,
            makeup: None,
            created_at: t0(),
            deadline_at: t0() + Duration::days(7),
        },
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
fn a_second_course_teacher_can_review_assignment() {
    let course_teacher = teacher(1);
    let reviewer = teacher(2);
    let group = student_group(GROUP_CODE);
    let single = course_for(&group, &course_teacher);
    let course = TeachingCourse::restore(
        single.id(),
        single.subject_id(),
        single.group_ids().clone(),
        HashSet::from([course_teacher.id(), reviewer.id()]),
        single.created_at(),
    )
    .unwrap();
    let student = student(3, &group);
    let assignment = assignment(&course, &course_teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .accept(
            &assignment,
            &course,
            &reviewer,
            None,
            "95/100",
            reviewed_at(),
        )
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
fn without_a_late_policy_the_deadline_moment_already_refuses_a_submission() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let deadline_at = assignment.deadline_at();
    let last_moment = deadline_at - Duration::nanoseconds(1);

    assert_eq!(
        submission(&assignment, &student)
            .submitted_at(deadline_at)
            .try_build()
            .unwrap_err(),
        DomainError::AssignmentDeadlineExceeded {
            deadline_at,
            submitted_at: deadline_at,
        }
    );
    assert_eq!(
        submission(&assignment, &student)
            .submitted_at(last_moment)
            .try_build()
            .map(|submission| submission.submitted_at()),
        Ok(last_moment)
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
fn a_deadline_must_lie_strictly_after_the_creation_moment() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
fn only_a_student_of_an_issued_group_submits_and_receives_extra_attempts() {
    let teacher = teacher(1);
    let issued = student_group(GROUP_CODE);
    let skipped = student_group(OTHER_GROUP_CODE);
    let course = teaching_course(&subject(SUBJECT_NAME), &[&issued, &skipped], &teacher);
    let assignment = assignment(&course, &teacher)
        .groups([&issued])
        .published()
        .build();

    for (group, expected) in [
        (&issued, Ok(())),
        (
            &skipped,
            Err(DomainError::StudentDoesNotBelongToAssignmentGroup),
        ),
    ] {
        let student = student(2, group);
        let code = group.code().as_str();

        assert_eq!(
            submission(&assignment, &student).try_build().map(drop),
            expected,
            "submission of {code}"
        );
        assert_eq!(
            StudentAssignmentProgress::new(&assignment, &student, 0).map(drop),
            expected,
            "extra attempts of {code}"
        );
    }
}

#[test]
fn staff_cannot_submit_homework() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    let assignment = assignment(&course, &teacher).published().build();

    for staff in [&teacher, &admin(2)] {
        assert_eq!(
            submission(&assignment, staff).try_build().unwrap_err(),
            DomainError::PermissionDenied {
                action: "submit homework"
            },
            "{:?}",
            staff.role()
        );
    }
}

#[test]
fn staff_receive_no_extra_attempts() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
fn a_student_of_another_group_learns_nothing_about_a_draft_or_a_closed_assignment() {
    let teacher = teacher(1);
    let assignment_group = student_group(GROUP_CODE);
    let other_group = student_group(OTHER_GROUP_CODE);
    let course = course_for(&assignment_group, &teacher);
    let student = student(2, &other_group);
    let draft = assignment(&course, &teacher).build();
    let closed = assignment(&course, &teacher)
        .deadline_at(t0() + Duration::days(1))
        .published()
        .build();

    for (assignment, at) in [(&draft, t0()), (&closed, t0() + Duration::days(2))] {
        let result = submission(assignment, &student)
            .submitted_at(at)
            .try_build();

        assert_eq!(
            result.unwrap_err(),
            DomainError::StudentDoesNotBelongToAssignmentGroup
        );
    }
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

    let empty_comment = submission.accept(&assignment, &course, &teacher, None, " ", reviewed_at());

    assert_eq!(
        empty_comment.unwrap_err(),
        DomainError::EmptyField {
            field: "submission.review_comment"
        }
    );
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            None,
            "95/100",
            reviewed_at(),
        )
        .unwrap();

    assert_eq!(submission.status(), SubmissionStatus::Accepted);
    assert_eq!(submission.review_comment().unwrap().as_str(), "95/100");
}

#[test]
fn a_rejection_requires_a_comment_too() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    assert_eq!(
        submission
            .reject(&assignment, &course, &teacher, " ", reviewed_at())
            .unwrap_err(),
        DomainError::EmptyField {
            field: "submission.review_comment"
        }
    );
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

#[test]
fn a_review_comment_is_bounded_by_the_user_text_limit() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let pending = submission(&assignment, &student).build();

    let mut longest = pending.clone();
    longest
        .reject(
            &assignment,
            &course,
            &teacher,
            "я".repeat(USER_TEXT_MAX_LEN),
            reviewed_at(),
        )
        .unwrap();
    let mut too_long = pending;
    assert_eq!(
        too_long
            .reject(
                &assignment,
                &course,
                &teacher,
                "я".repeat(USER_TEXT_MAX_LEN + 1),
                reviewed_at(),
            )
            .unwrap_err(),
        DomainError::FieldTooLong {
            field: "submission.review_comment",
            max: USER_TEXT_MAX_LEN,
            actual: USER_TEXT_MAX_LEN + 1,
        }
    );
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

    let second_review = submission.accept(
        &assignment,
        &course,
        &teacher,
        None,
        "95/100",
        reviewed_at(),
    );
    assert_eq!(
        second_review.unwrap_err(),
        DomainError::SubmissionIsNotPendingReview
    );
}

fn rejection_with_a_score() -> Review {
    Review {
        decision: ReviewDecision::Reject,
        score: out_of(4, 5),
        comment: "Нет тестов".to_owned(),
        reviewed_at: reviewed_at(),
    }
}

#[test]
fn a_rejection_with_a_score_from_an_outsider_is_refused_for_access_first() {
    let outsider = teacher(3);
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.review(&assignment, &course, &outsider, rejection_with_a_score());

    assert_eq!(
        result,
        Err(DomainError::PermissionDenied {
            action: "review homework"
        })
    );
}

#[test]
fn a_rejection_with_a_score_of_a_finished_review_reports_the_finished_review() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();
    submission
        .reject(&assignment, &course, &teacher, "Нет тестов", reviewed_at())
        .unwrap();

    let result = submission.review(&assignment, &course, &teacher, rejection_with_a_score());

    assert_eq!(result, Err(DomainError::SubmissionIsNotPendingReview));
}

#[test]
fn a_rejection_carries_no_score() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.review(&assignment, &course, &teacher, rejection_with_a_score());

    assert_eq!(result, Err(DomainError::ScoreNotAllowed));
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

fn weekly_penalty_assignment(
    course: &TeachingCourse,
    teacher: &User,
) -> (Assignment, DateTime<Utc>) {
    let deadline_at = Utc.with_ymd_and_hms(2026, 8, 24, 23, 59, 0).unwrap();
    let assignment = assignment(course, teacher)
        .created_at(Utc.with_ymd_and_hms(2026, 8, 17, 13, 30, 0).unwrap())
        .deadline_at(deadline_at)
        .scored(5)
        .late_policy(7, 1, 4, DeductedFrom::WorkScore)
        .published()
        .build();

    (assignment, deadline_at)
}

#[test]
fn the_deadline_moment_itself_is_already_late() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    let just_before = deadline_at - Duration::nanoseconds(1);
    assert_eq!(assignment.late_periods_at(just_before), 0);
    assert_eq!(assignment.late_penalty_at(just_before), None);
    assert_eq!(assignment.late_periods_at(deadline_at), 1);
    assert_eq!(points_at(&assignment, deadline_at), Some(1));
}

#[test]
fn creating_an_assignment_rejects_a_horizon_longer_than_366_days() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
fn any_lateness_costs_the_first_period_at_once() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let at = Utc.with_ymd_and_hms(2026, 8, 25, 0, 0, 0).unwrap();

    assert_eq!(assignment.late_periods_at(at), 1);
    assert_eq!(points_at(&assignment, at), Some(1));
}

#[test]
fn exactly_366_days_of_lateness_are_allowed() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    let work = assignment(&course, &teacher)
        .scored(10)
        .late_policy(183, 1, 2, DeductedFrom::WorkScore)
        .build();
    assert_eq!(work.closes_at(), work.deadline_at() + Duration::days(366));
}

#[test]
fn the_shortest_representable_horizon_over_366_days_is_rejected() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);

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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
fn zero_possible_score_does_not_close_submissions_early() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    let submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(1))
        .try_build();

    assert!(submission.is_ok());
}

#[test]
fn a_review_freezes_the_lateness_of_the_submission_not_of_the_review() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let mut submission = submission(&assignment, &student(2, &group))
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
fn moving_the_deadline_shifts_the_closing_date() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (mut assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let closes_at = assignment.closes_at();

    assignment
        .change_deadline(&course, &teacher, deadline_at + Duration::days(7))
        .unwrap();

    assert_eq!(assignment.closes_at(), closes_at + Duration::days(7));
}

#[test]
fn a_late_policy_needs_a_scored_scale() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);

    let result = assignment(&course, &teacher)
        .late_policy(7, 1, 4, DeductedFrom::WorkScore)
        .try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::LatePolicyRequiresScoredGrading
    );
}

#[test]
fn nonweekly_closing_is_an_exact_exclusive_cutoff() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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

#[test]
fn deadline_and_policy_can_change_atomically() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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

#[test]
fn accepting_scored_assignment_without_score_is_rejected() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(&assignment, &course, &teacher, None, "Ок", reviewed_at());

    assert_eq!(result.unwrap_err(), DomainError::ScoreRequired);
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

#[test]
fn score_above_the_scale_is_rejected() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(
        &assignment,
        &course,
        &teacher,
        out_of(6, 5),
        "Ок",
        reviewed_at(),
    );

    assert_eq!(
        result.unwrap_err(),
        DomainError::ScoreOutOfRange { value: 6, max: 5 }
    );
}

#[test]
fn score_on_pass_fail_assignment_is_rejected() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(
        &assignment,
        &course,
        &teacher,
        out_of(1, 5),
        "Ок",
        reviewed_at(),
    );

    assert_eq!(result.unwrap_err(), DomainError::ScoreNotAllowed);
}

#[test]
fn accepted_scored_submission_keeps_the_score_of_the_reviewer() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            out_of(4, 5),
            "Ок",
            reviewed_at(),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.raw_score(), 4);
    assert_eq!(grade.work_score(), 4);
}

#[test]
fn rejected_submission_of_scored_assignment_has_no_grade() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .reject(&assignment, &course, &teacher, "Нет тестов", reviewed_at())
        .unwrap();

    assert_eq!(submission.grade(), None);
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
        None,
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

    let result = submission.accept(
        &assignment,
        &course,
        &student,
        None,
        "95/100",
        reviewed_at(),
    );

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

    let result = submission.accept(&other, &course, &teacher, None, "95/100", reviewed_at());

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
        MakeupRequest::default(),
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
fn a_new_attempt_is_offered_only_where_submit_would_succeed() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let draft = assignment(&course, &teacher).max_attempts(2).build();
    let assignment = assignment(&course, &teacher)
        .max_attempts(2)
        .published()
        .build();
    let at = t0();

    assert!(!history().build().accepts_new_attempt_for(&draft, at));
    assert!(history().build().accepts_new_attempt_for(&assignment, at));
    assert!(
        history()
            .used(1)
            .build()
            .accepts_new_attempt_for(&assignment, at)
    );
    assert!(
        !history()
            .used(2)
            .build()
            .accepts_new_attempt_for(&assignment, at)
    );
    assert!(
        !history()
            .pending()
            .build()
            .accepts_new_attempt_for(&assignment, at)
    );
    assert!(
        !history()
            .accepted()
            .build()
            .accepts_new_attempt_for(&assignment, at)
    );
    assert!(
        !history()
            .build()
            .accepts_new_attempt_for(&assignment, assignment.closes_at())
    );
}

#[test]
fn a_regular_attempt_is_refused_by_the_work_state_or_a_closed_window() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let foreign_course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let (teacher, outsider) = (teacher(1), teacher(2));
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let work = credit_assignment(&course, &teacher);

    assert_eq!(
        submission(&work, &student)
            .history(history().submissions(crate::domain::MAX_ATTEMPTS).build())
            .submitted_at(after_closing(&work))
            .makeup(makeup_with_used(0))
            .try_build(),
        Err(DomainError::AttemptLimitExceeded {
            value: u32::from(crate::domain::MAX_ATTEMPTS) + 1,
            max: crate::domain::MAX_ATTEMPTS,
        })
    );
}

#[test]
fn the_card_offers_a_makeup_with_its_balance_or_nothing() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
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

#[test]
fn a_makeup_is_credited_by_the_terms_at_review_without_a_penalty() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
        makeup.accept(&work, &course, &teacher, out_of(6, 10), "Ок", reviewed_at()),
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
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
