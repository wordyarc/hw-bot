use std::collections::HashSet;

use super::{Fixture, fixture, out_of, reviewed_at};
use crate::domain::{
    DomainError, Review, ReviewDecision, SubmissionStatus, TeachingCourse, USER_TEXT_MAX_LEN,
};
use crate::test_support::{ReviewShortcuts, admin, assignment, submission, teacher};

#[test]
fn a_second_course_teacher_can_review_assignment() {
    let Fixture {
        teacher: course_teacher,
        course: single,
        student,
        ..
    } = fixture();
    let reviewer = teacher(2);
    let course = TeachingCourse::restore(
        single.id(),
        single.subject_id(),
        single.group_ids().clone(),
        HashSet::from([course_teacher.id(), reviewer.id()]),
        single.created_at(),
    )
    .unwrap();
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
fn accepted_review_requires_comment_and_stores_it() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.review(&assignment, &course, &teacher, rejection_with_a_score());

    assert_eq!(result, Err(DomainError::ScoreNotAllowed));
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

#[test]
fn accepting_scored_assignment_without_score_is_rejected() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    let result = submission.accept(&assignment, &course, &teacher, None, "Ок", reviewed_at());

    assert_eq!(result.unwrap_err(), DomainError::ScoreRequired);
    assert_eq!(submission.status(), SubmissionStatus::PendingReview);
}

#[test]
fn accepted_scored_submission_keeps_the_score_of_the_reviewer() {
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
    let assignment = assignment(&course, &teacher).scored(5).published().build();
    let mut submission = submission(&assignment, &student).build();

    submission
        .reject(&assignment, &course, &teacher, "Нет тестов", reviewed_at())
        .unwrap();

    assert_eq!(submission.grade(), None);
}

#[test]
fn only_course_teacher_or_admin_can_review_submission() {
    let Fixture {
        teacher: course_teacher,
        course,
        student,
        ..
    } = fixture();
    let other_teacher = teacher(2);
    let admin = admin(3);
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
    let Fixture {
        teacher,
        course,
        student,
        ..
    } = fixture();
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
