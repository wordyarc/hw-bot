use chrono::Duration;

use super::{Fixture, fixture};
use crate::domain::{
    DomainError, MakeupRequest, StudentAssignmentProgress, Submission, SubmissionHistory,
    SubmissionId,
};
use crate::test_support::{
    OTHER_GROUP_CODE, PULL_REQUEST_URL, SUBJECT_NAME, admin, assignment, history, student,
    student_group, subject, submission, t0, teaching_course,
};

#[test]
fn student_cannot_submit_assignment_after_deadline() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
fn student_from_another_group_cannot_submit_assignment() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let student = student(2, &student_group(OTHER_GROUP_CODE));
    let assignment = assignment(&course, &teacher).published().build();

    let result = submission(&assignment, &student).try_build();

    assert_eq!(
        result.unwrap_err(),
        DomainError::StudentDoesNotBelongToAssignmentGroup
    );
}

#[test]
fn only_a_student_of_an_issued_group_submits_and_receives_extra_attempts() {
    let Fixture {
        teacher,
        group: issued,
        ..
    } = fixture();
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
    let Fixture {
        teacher, course, ..
    } = fixture();
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
fn a_student_of_another_group_learns_nothing_about_a_draft_or_a_closed_assignment() {
    let Fixture {
        teacher, course, ..
    } = fixture();
    let student = student(2, &student_group(OTHER_GROUP_CODE));
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let draft = assignment(&course, &teacher).build();

    let result = submission(&draft, &student).try_build();

    assert_eq!(result.unwrap_err(), DomainError::AssignmentIsNotPublished);
}

#[test]
fn accepted_assignment_cannot_be_submitted_again() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
fn submission_content_is_trimmed() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
