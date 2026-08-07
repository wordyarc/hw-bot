use chrono::{DateTime, Utc};

use crate::domain::enums::str_enum;
use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, NonEmptyString, SubmissionId,
    TeachingCourse, User, UserId,
};

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
        assignment
            .max_attempts()
            .saturating_add(self.extra_attempts)
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
    /// Решение ревьюера, а не результат проверки: результат - это статус самой сдачи.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ReviewDecision {
        Accept => "accept",
        Reject => "reject",
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
}

/// Поля сдачи россыпью: `restore` собирает сущность из них по именам, чтобы соседние
/// `DateTime<Utc>` нельзя было переставить местами незаметно для компилятора.
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

        Ok(Self {
            id: parts.id,
            assignment_id: parts.assignment_id,
            student_id: parts.student_id,
            attempt_number: parts.attempt_number,
            content: NonEmptyString::new("submission.content", parts.content)?,
            status: parts.status,
            submitted_at: parts.submitted_at,
            reviewed_by: parts.reviewed_by,
            reviewed_at: parts.reviewed_at,
            review_comment: parts
                .review_comment
                .map(|comment| NonEmptyString::new("submission.review_comment", comment))
                .transpose()?,
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

        if submitted_at > assignment.deadline_at() {
            return Err(DomainError::AssignmentDeadlineExceeded {
                deadline_at: assignment.deadline_at(),
                submitted_at,
            });
        }

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
        })
    }

    pub fn accept(
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
            SubmissionStatus::Accepted,
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
            comment,
            reviewed_at,
        )
    }

    fn complete_review(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        status: SubmissionStatus,
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

        let review_comment = NonEmptyString::new("submission.review_comment", comment)?;

        self.status = status;
        self.reviewed_by = Some(reviewer.id());
        self.reviewed_at = Some(reviewed_at);
        self.review_comment = Some(review_comment);

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

    pub fn is_pending_review(&self) -> bool {
        self.status == SubmissionStatus::PendingReview
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_parts() -> SubmissionParts {
        SubmissionParts {
            id: SubmissionId::new(),
            assignment_id: AssignmentId::new(),
            student_id: UserId::new(),
            attempt_number: 1,
            content: "https://github.com/course/hw/pull/1".to_owned(),
            status: SubmissionStatus::PendingReview,
            submitted_at: Utc::now(),
            reviewed_by: None,
            reviewed_at: None,
            review_comment: None,
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
    fn restore_rejects_review_fields_for_pending_submission() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                reviewed_by: Some(UserId::new()),
                reviewed_at: Some(Utc::now()),
                review_comment: Some("review".to_owned()),
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionReviewState
        );
    }
}
