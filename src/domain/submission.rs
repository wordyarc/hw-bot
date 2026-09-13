use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, MAX_SCORE, NonEmptyString, SubmissionId,
    TeachingCourse, USER_TEXT_MAX_LEN, User, UserId,
};
use crate::string_enum::str_enum;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubmissionHistory {
    used_attempts: u16,
    extra_attempts: u16,
    has_accepted_submission: bool,
    has_pending_submission: bool,
}

impl SubmissionHistory {
    pub fn new(
        used_attempts: u16,
        extra_attempts: u16,
        has_accepted_submission: bool,
        has_pending_submission: bool,
    ) -> Self {
        Self {
            used_attempts,
            extra_attempts,
            has_accepted_submission,
            has_pending_submission,
        }
    }

    pub fn used_attempts(self) -> u16 {
        self.used_attempts
    }

    pub fn extra_attempts(self) -> u16 {
        self.extra_attempts
    }

    pub fn has_accepted_submission(self) -> bool {
        self.has_accepted_submission
    }

    pub fn has_pending_submission(self) -> bool {
        self.has_pending_submission
    }

    pub fn allowed_attempts_for(self, assignment: &Assignment) -> u16 {
        assignment.max_attempts() + self.extra_attempts
    }

    /// Сколько попыток бот показывает студенту. Принятая сдача и
    /// сдача на проверке блокируют следующую попытку, даже когда остаток положительный.
    pub fn remaining_attempts_for(self, assignment: &Assignment) -> u16 {
        self.allowed_attempts_for(assignment)
            .saturating_sub(self.used_attempts)
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SubmissionStatus {
        PendingReview => "pending_review",
        Accepted => "accepted",
        Rejected => "rejected",
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ReviewDecision {
        Accept => "accept",
        Reject => "reject",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grade {
    raw_score: u16,
    max_score: u16,
    late_periods: u32,
    penalty: u16,
    final_score: u16,
}

impl Grade {
    pub(crate) fn new(raw_score: u16, max_score: u16, late_periods: u32, penalty: u16) -> Self {
        Self {
            raw_score,
            max_score,
            late_periods,
            penalty,
            // Штраф может превышать балл: итог зажимается в ноль, а не уходит в минус.
            final_score: raw_score.saturating_sub(penalty),
        }
    }

    fn restore(
        raw_score: u16,
        max_score: u16,
        late_periods: u32,
        penalty: u16,
        final_score: u16,
    ) -> DomainResult<Self> {
        if max_score == 0
            || max_score > MAX_SCORE
            || raw_score > max_score
            || penalty > MAX_SCORE
            || final_score != raw_score.saturating_sub(penalty)
        {
            return Err(DomainError::InvalidSubmissionGradeState);
        }

        Ok(Self {
            raw_score,
            max_score,
            late_periods,
            penalty,
            final_score,
        })
    }

    pub fn raw_score(self) -> u16 {
        self.raw_score
    }

    pub fn max_score(self) -> u16 {
        self.max_score
    }

    pub fn late_periods(self) -> u32 {
        self.late_periods
    }

    pub fn penalty(self) -> u16 {
        self.penalty
    }

    pub fn final_score(self) -> u16 {
        self.final_score
    }
}

#[cfg(test)]
mod grade_restore_tests {
    use super::*;

    #[test]
    fn rejects_a_stored_grade_with_inconsistent_arithmetic() {
        assert_eq!(
            Grade::restore(4, 5, 2, 2, 4),
            Err(DomainError::InvalidSubmissionGradeState)
        );
    }

    #[test]
    fn rejects_a_stored_penalty_above_the_effective_limit() {
        assert_eq!(
            Grade::restore(4, 5, 2, 101, 0),
            Err(DomainError::InvalidSubmissionGradeState)
        );
    }

    #[test]
    fn restores_a_grade_clamped_to_zero() {
        assert_eq!(Grade::restore(4, 5, 2, 100, 0).unwrap().final_score(), 0);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submission {
    id: SubmissionId,
    assignment_id: AssignmentId,
    student_id: UserId,
    attempt_number: u16,
    content: NonEmptyString,
    status: SubmissionStatus,
    submitted_at: DateTime<Utc>,
    reviewed_by: Option<UserId>,
    reviewed_at: Option<DateTime<Utc>>,
    review_comment: Option<NonEmptyString>,
    grade: Option<Grade>,
}

pub(crate) struct SubmissionParts {
    pub id: SubmissionId,
    pub assignment_id: AssignmentId,
    pub student_id: UserId,
    pub attempt_number: u16,
    pub content: String,
    pub status: SubmissionStatus,
    pub submitted_at: DateTime<Utc>,
    pub reviewed_by: Option<UserId>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub review_comment: Option<String>,
    pub raw_score: Option<u16>,
    pub grade_max_score: Option<u16>,
    pub late_periods: Option<u32>,
    pub penalty: Option<u16>,
    pub final_score: Option<u16>,
}

impl Submission {
    pub(crate) fn restore(parts: SubmissionParts) -> DomainResult<Self> {
        if parts.attempt_number == 0 {
            return Err(DomainError::InvalidSubmissionAttemptNumber {
                value: parts.attempt_number,
            });
        }

        match parts.status {
            SubmissionStatus::PendingReview
                if parts.reviewed_by.is_some()
                    || parts.reviewed_at.is_some()
                    || parts.review_comment.is_some() =>
            {
                return Err(DomainError::InvalidSubmissionReviewState);
            }
            SubmissionStatus::Accepted | SubmissionStatus::Rejected
                if parts.reviewed_by.is_none()
                    || parts.reviewed_at.is_none()
                    || parts.review_comment.is_none() =>
            {
                return Err(DomainError::InvalidSubmissionReviewState);
            }
            _ => {}
        }

        let grade = match (
            parts.raw_score,
            parts.grade_max_score,
            parts.late_periods,
            parts.penalty,
            parts.final_score,
        ) {
            (None, None, None, None, None) => None,
            (
                Some(raw_score),
                Some(max_score),
                Some(late_periods),
                Some(penalty),
                Some(final_score),
            ) => {
                if parts.status != SubmissionStatus::Accepted {
                    return Err(DomainError::InvalidSubmissionGradeState);
                }

                Some(Grade::restore(
                    raw_score,
                    max_score,
                    late_periods,
                    penalty,
                    final_score,
                )?)
            }
            _ => return Err(DomainError::InvalidSubmissionGradeState),
        };

        Ok(Self {
            id: parts.id,
            assignment_id: parts.assignment_id,
            student_id: parts.student_id,
            attempt_number: parts.attempt_number,
            content: NonEmptyString::new_bounded(
                "submission.content",
                parts.content,
                USER_TEXT_MAX_LEN,
            )?,
            status: parts.status,
            submitted_at: parts.submitted_at,
            reviewed_by: parts.reviewed_by,
            reviewed_at: parts.reviewed_at,
            review_comment: parts
                .review_comment
                .map(|comment| {
                    NonEmptyString::new_bounded(
                        "submission.review_comment",
                        comment,
                        USER_TEXT_MAX_LEN,
                    )
                })
                .transpose()?,
            grade,
        })
    }

    pub fn submit(
        id: SubmissionId,
        assignment: &Assignment,
        student: &User,
        history: SubmissionHistory,
        content: impl Into<String>,
        submitted_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "submit homework",
            });
        }

        if !assignment.is_published() {
            return Err(DomainError::AssignmentIsNotPublished);
        }

        assignment.ensure_open_for_submission(submitted_at)?;

        if student.student_group_id() != Some(assignment.student_group_id()) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup);
        }

        if history.has_accepted_submission {
            return Err(DomainError::SubmissionAlreadyAccepted);
        }

        if history.has_pending_submission {
            return Err(DomainError::SubmissionAlreadyPendingReview);
        }

        let allowed = history.allowed_attempts_for(assignment);
        if history.used_attempts >= allowed {
            return Err(DomainError::AttemptsExhausted {
                allowed,
                used: history.used_attempts,
            });
        }

        Self::restore(SubmissionParts {
            id,
            assignment_id: assignment.id(),
            student_id: student.id(),
            attempt_number: history.used_attempts + 1,
            content: content.into(),
            status: SubmissionStatus::PendingReview,
            submitted_at,
            reviewed_by: None,
            reviewed_at: None,
            review_comment: None,
            raw_score: None,
            grade_max_score: None,
            late_periods: None,
            penalty: None,
            final_score: None,
        })
    }

    pub fn accept(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        score: Option<u16>,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            teaching_course,
            reviewer,
            SubmissionStatus::Accepted,
            score,
            comment,
            reviewed_at,
        )
    }

    pub fn reject(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            teaching_course,
            reviewer,
            SubmissionStatus::Rejected,
            None,
            comment,
            reviewed_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_review(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        status: SubmissionStatus,
        score: Option<u16>,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        if self.assignment_id != assignment.id() {
            return Err(DomainError::SubmissionDoesNotBelongToAssignment);
        }

        assignment.ensure_belongs_to_course(teaching_course)?;

        if !teaching_course.can_be_managed_by(reviewer) {
            return Err(DomainError::PermissionDenied {
                action: "review homework",
            });
        }

        if self.status != SubmissionStatus::PendingReview {
            return Err(DomainError::SubmissionIsNotPendingReview);
        }

        let grade = match status {
            SubmissionStatus::Accepted => {
                // Опоздание считается по моменту отправки: скорость работы преподавателя не должна
                // менять оценку студента.
                let late_periods = assignment.late_periods_at(self.submitted_at);
                let penalty = assignment.penalty_at(self.submitted_at);
                let max_score = assignment.grading().max_score();

                assignment
                    .grading()
                    .validate_score(score)?
                    .map(|raw_score| {
                        Grade::new(
                            raw_score,
                            max_score.expect("a validated score requires scored grading"),
                            late_periods,
                            penalty,
                        )
                    })
            }
            _ => None,
        };

        let review_comment =
            NonEmptyString::new_bounded("submission.review_comment", comment, USER_TEXT_MAX_LEN)?;

        self.status = status;
        self.reviewed_by = Some(reviewer.id());
        self.reviewed_at = Some(reviewed_at);
        self.review_comment = Some(review_comment);
        self.grade = grade;

        Ok(())
    }

    pub fn id(&self) -> SubmissionId {
        self.id
    }

    pub fn assignment_id(&self) -> AssignmentId {
        self.assignment_id
    }

    pub fn student_id(&self) -> UserId {
        self.student_id
    }

    pub fn attempt_number(&self) -> u16 {
        self.attempt_number
    }

    pub fn content(&self) -> &NonEmptyString {
        &self.content
    }

    pub fn status(&self) -> SubmissionStatus {
        self.status
    }

    pub fn submitted_at(&self) -> DateTime<Utc> {
        self.submitted_at
    }

    pub fn reviewed_by(&self) -> Option<UserId> {
        self.reviewed_by
    }

    pub fn reviewed_at(&self) -> Option<DateTime<Utc>> {
        self.reviewed_at
    }

    pub fn review_comment(&self) -> Option<&NonEmptyString> {
        self.review_comment.as_ref()
    }

    pub fn grade(&self) -> Option<Grade> {
        self.grade
    }

    pub fn is_pending_review(&self) -> bool {
        self.status == SubmissionStatus::PendingReview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::t0;

    fn pending_parts() -> SubmissionParts {
        SubmissionParts {
            id: SubmissionId::new(),
            assignment_id: AssignmentId::new(),
            student_id: UserId::new(),
            attempt_number: 1,
            content: "https://github.com/course/hw/pull/1".to_owned(),
            status: SubmissionStatus::PendingReview,
            submitted_at: t0(),
            reviewed_by: None,
            reviewed_at: None,
            review_comment: None,
            raw_score: None,
            grade_max_score: None,
            late_periods: None,
            penalty: None,
            final_score: None,
        }
    }

    fn accepted_parts() -> SubmissionParts {
        SubmissionParts {
            status: SubmissionStatus::Accepted,
            reviewed_by: Some(UserId::new()),
            reviewed_at: Some(t0()),
            review_comment: Some("Good".to_owned()),
            ..pending_parts()
        }
    }

    #[test]
    fn restore_accepts_pending_submission_without_review_fields() {
        let submission = Submission::restore(pending_parts()).unwrap();

        assert_eq!(submission.status(), SubmissionStatus::PendingReview);
        assert_eq!(submission.attempt_number(), 1);
    }

    #[test]
    fn restore_rejects_zero_attempt_number() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                attempt_number: 0,
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionAttemptNumber { value: 0 }
        );
    }

    #[test]
    fn restore_requires_review_fields_for_completed_submission() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                status: SubmissionStatus::Accepted,
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionReviewState
        );
    }

    #[test]
    fn restore_rejects_partial_grade() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                raw_score: Some(4),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_rejects_grade_on_submission_that_is_not_accepted() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                status: SubmissionStatus::Rejected,
                raw_score: Some(4),
                grade_max_score: Some(5),
                late_periods: Some(0),
                penalty: Some(0),
                final_score: Some(4),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_rejects_final_score_above_raw_score() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                raw_score: Some(4),
                grade_max_score: Some(5),
                late_periods: Some(0),
                penalty: Some(0),
                final_score: Some(5),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_rejects_raw_score_above_the_frozen_scale() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                raw_score: Some(4),
                grade_max_score: Some(3),
                late_periods: Some(0),
                penalty: Some(0),
                final_score: Some(4),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_accepts_full_grade_of_accepted_submission() {
        let grade = Submission::restore(SubmissionParts {
            raw_score: Some(4),
            grade_max_score: Some(5),
            late_periods: Some(2),
            penalty: Some(2),
            final_score: Some(2),
            ..accepted_parts()
        })
        .unwrap()
        .grade()
        .unwrap();

        assert_eq!(grade.raw_score(), 4);
        assert_eq!(grade.max_score(), 5);
        assert_eq!(grade.late_periods(), 2);
        assert_eq!(grade.penalty(), 2);
        assert_eq!(grade.final_score(), 2);
    }

    #[test]
    fn grade_clamps_final_score_at_zero() {
        let grade = Grade::new(4, 5, 5, 5);

        assert_eq!(grade.final_score(), 0);
        assert_eq!(grade.raw_score(), 4);
        assert_eq!(grade.penalty(), 5);
    }

    #[test]
    fn restore_rejects_review_fields_for_pending_submission() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                reviewed_by: Some(UserId::new()),
                reviewed_at: Some(t0()),
                review_comment: Some("review".to_owned()),
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionReviewState
        );
    }
}
