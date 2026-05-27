use chrono::{DateTime, Utc};

use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, ExtraAttemptGrantId, NonEmptyString,
    TeachingCourse, User, UserId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtraAttemptGrant {
    id: ExtraAttemptGrantId,
    assignment_id: AssignmentId,
    student_id: UserId,
    granted_by: UserId,
    amount: u16,
    reason: Option<NonEmptyString>,
    granted_at: DateTime<Utc>,
}

impl ExtraAttemptGrant {
    pub fn new(
        id: ExtraAttemptGrantId,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        student: &User,
        granted_by: &User,
        amount: u16,
        reason: Option<String>,
        granted_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "receive extra attempts",
            });
        }

        if student.student_group_id() != Some(assignment.student_group_id()) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup);
        }

        assignment.ensure_belongs_to_course(teaching_course)?;

        if !teaching_course.can_be_managed_by(granted_by) {
            return Err(DomainError::PermissionDenied {
                action: "grant extra attempts",
            });
        }

        if amount == 0 {
            return Err(DomainError::InvalidAttemptGrant { value: amount });
        }

        Ok(Self {
            id,
            assignment_id: assignment.id(),
            student_id: student.id(),
            granted_by: granted_by.id(),
            amount,
            reason: reason
                .map(|value| NonEmptyString::new("extra_attempt_grant.reason", value))
                .transpose()?,
            granted_at,
        })
    }

    pub fn id(&self) -> ExtraAttemptGrantId {
        self.id
    }

    pub fn assignment_id(&self) -> AssignmentId {
        self.assignment_id
    }

    pub fn student_id(&self) -> UserId {
        self.student_id
    }

    pub fn granted_by(&self) -> UserId {
        self.granted_by
    }

    pub fn amount(&self) -> u16 {
        self.amount
    }

    pub fn reason(&self) -> Option<&NonEmptyString> {
        self.reason.as_ref()
    }

    pub fn granted_at(&self) -> DateTime<Utc> {
        self.granted_at
    }
}
