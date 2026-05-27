use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, NonEmptyString, SubmissionId, User, UserId,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionStatus {
    PendingReview,
    Accepted,
    Rejected,
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

impl Submission {
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

        Ok(Self {
            id,
            assignment_id: assignment.id(),
            student_id: student.id(),
            attempt_number: history.used_attempts + 1,
            content: NonEmptyString::new("submission.content", content)?,
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
        reviewer: &User,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            reviewer,
            SubmissionStatus::Accepted,
            comment,
            reviewed_at,
        )
    }

    pub fn reject(
        &mut self,
        assignment: &Assignment,
        reviewer: &User,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            reviewer,
            SubmissionStatus::Rejected,
            comment,
            reviewed_at,
        )
    }

    fn complete_review(
        &mut self,
        assignment: &Assignment,
        reviewer: &User,
        status: SubmissionStatus,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        if self.assignment_id != assignment.id() {
            return Err(DomainError::SubmissionDoesNotBelongToAssignment);
        }

        if !assignment.can_be_managed_by(reviewer) {
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
