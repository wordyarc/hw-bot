use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, DomainError, Grading, LatePolicy, NewAssignment,
    Role, StaffRole, StudentAssignmentProgress, Submission, SubmissionHistory, SubmissionId,
    SubmissionStatus, TeachingCourse, TelegramUserId, User, UserId,
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
        NewAssignment {
            title: ASSIGNMENT_TITLE.to_owned(),
            body: ASSIGNMENT_BODY.to_owned(),
            max_attempts: 2,
            grading: Grading::PassFail,
            late_policy: None,
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

fn weekly_penalty_assignment(
    course: &TeachingCourse,
    teacher: &User,
) -> (Assignment, DateTime<Utc>) {
    let deadline_at = Utc.with_ymd_and_hms(2026, 8, 24, 23, 59, 0).unwrap();
    let assignment = assignment(course, teacher)
        .created_at(Utc.with_ymd_and_hms(2026, 8, 17, 13, 30, 0).unwrap())
        .deadline_at(deadline_at)
        .scored(5)
        .late_policy(7, 1, 4)
        .published()
        .build();

    (assignment, deadline_at)
}

#[test]
fn a_submission_at_the_deadline_is_not_late() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    assert_eq!(assignment.late_periods_at(deadline_at), 0);
    assert_eq!(assignment.penalty_at(deadline_at), 0);
}

#[test]
fn creating_an_assignment_rejects_a_horizon_longer_than_366_days() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    assert_eq!(
        assignment(&course, &teacher)
            .scored(5)
            .late_policy(365, 1, 100)
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
        .late_policy(1, 1, 2)
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
fn adding_a_policy_rejects_date_overflow_without_changing_the_scale() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    let mut work = assignment(&course, &teacher)
        .deadline_at(DateTime::<Utc>::MAX_UTC)
        .build();
    let before = work.clone();
    assert_eq!(
        work.change_grading(
            &course,
            &teacher,
            Grading::scored(5).unwrap(),
            Some(LatePolicy::new(1, 1, 1).unwrap())
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
    assert_eq!(assignment.penalty_at(at), 1);
}

#[test]
fn exactly_366_days_of_lateness_are_allowed() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    let work = assignment(&course, &teacher)
        .scored(10)
        .late_policy(183, 1, 2)
        .build();
    assert_eq!(work.closes_at(), work.deadline_at() + Duration::days(366));
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
        Some(LatePolicy::new(7, 2, 4).unwrap()),
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
        Some(LatePolicy::new(10, 1, 4).unwrap()),
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
            .late_policy(1, 1, 1)
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
        .late_policy(7, 1, 4)
        .published()
        .build();
    assert_eq!(work.penalty_at(work.closes_at()), 4);
    assert!(
        submission(&work, &student)
            .submitted_at(work.closes_at())
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

    let last_of_the_first = Utc.with_ymd_and_hms(2026, 8, 31, 23, 59, 0).unwrap();

    assert_eq!(assignment.late_periods_at(last_of_the_first), 1);
    assert_eq!(
        assignment.late_periods_at(last_of_the_first + Duration::seconds(1)),
        2
    );
}

#[test]
fn fractional_seconds_after_each_period_boundary_count_as_the_next_period() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);

    for period in 0..4 {
        let boundary = deadline_at + Duration::weeks(period);
        assert_eq!(assignment.late_periods_at(boundary), period as u32);
        for fraction in [Duration::nanoseconds(1), Duration::milliseconds(500)] {
            assert_eq!(
                assignment.late_periods_at(boundary + fraction),
                period as u32 + 1
            );
        }
    }
}

#[test]
fn the_horizon_is_open_at_its_own_moment_and_closed_after_it() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let (assignment, _) = weekly_penalty_assignment(&course, &teacher);

    let closes_at = Utc.with_ymd_and_hms(2026, 9, 21, 23, 59, 0).unwrap();
    assert_eq!(assignment.closes_at(), closes_at);

    assignment.ensure_open_for_submission(closes_at).unwrap();
    assert_eq!(assignment.late_periods_at(closes_at), 4);
    assert_eq!(
        assignment
            .ensure_open_for_submission(closes_at + Duration::seconds(1))
            .unwrap_err(),
        DomainError::SubmissionWindowClosed {
            closes_at,
            submitted_at: closes_at + Duration::seconds(1),
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
            Some(5),
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
    assert_eq!(grade.final_score(), 4);
}

#[test]
fn a_penalty_larger_than_the_score_leaves_an_accepted_submission_with_zero() {
    let teacher = teacher(1);
    let group = student_group(GROUP_CODE);
    let course = course_for(&group, &teacher);
    let student = student(2, &group);
    let (assignment, deadline_at) = weekly_penalty_assignment(&course, &teacher);
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(28))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            Some(4),
            "Ок",
            deadline_at + Duration::days(29),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.raw_score(), 4);
    assert_eq!(grade.max_score(), 5);
    assert_eq!(grade.penalty(), 4);
    assert_eq!(grade.final_score(), 0);
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
        .late_policy(1, 100, 2)
        .published()
        .build();
    let mut submission = submission(&assignment, &student)
        .submitted_at(deadline_at + Duration::days(2))
        .build();

    submission
        .accept(
            &assignment,
            &course,
            &teacher,
            Some(100),
            "Ок",
            deadline_at + Duration::days(3),
        )
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.max_score(), 100);
    assert_eq!(grade.late_periods(), 2);
    assert_eq!(grade.penalty(), 100);
    assert_eq!(grade.final_score(), 0);
    assert_eq!(submission.status(), SubmissionStatus::Accepted);
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
        .late_policy(7, 1, 4)
        .try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::LatePolicyRequiresScoredGrading
    );
}

#[test]
fn nonweekly_closing_is_an_exact_inclusive_cutoff() {
    let teacher = teacher(1);
    let course = course_for(&student_group(GROUP_CODE), &teacher);
    let deadline = t0() + Duration::days(7);
    let closing = deadline + Duration::days(10);
    let work = assignment(&course, &teacher)
        .scored(5)
        .deadline_at(deadline)
        .late_policy(5, 1, 2)
        .build();
    assert_eq!(work.closes_at(), closing);
    assert_eq!(work.penalty_at(closing), 2);
    assert!(work.ensure_open_for_submission(closing).is_ok());
    assert!(matches!(
        work.ensure_open_for_submission(closing + Duration::nanoseconds(1)),
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
        Some(LatePolicy::new(5, 1, 2).unwrap()),
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
            Some(LatePolicy::new(365, 1, 2).unwrap())
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
            Some(LatePolicy::new(1, 1, 2).unwrap())
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

    let result = submission.accept(&assignment, &course, &teacher, Some(6), "Ок", reviewed_at());

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

    let result = submission.accept(&assignment, &course, &teacher, Some(1), "Ок", reviewed_at());

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
        .accept(&assignment, &course, &teacher, Some(4), "Ок", reviewed_at())
        .unwrap();

    let grade = submission.grade().unwrap();
    assert_eq!(grade.raw_score(), 4);
    assert_eq!(grade.final_score(), 4);
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
