use crate::domain::assignment::MAX_ATTEMPTS;
use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, TeachingCourse, User, UserId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StudentAssignmentProgress {
    assignment_id: AssignmentId,
    student_id: UserId,
    extra_attempts: u16,
}

impl StudentAssignmentProgress {
    pub fn new(assignment: &Assignment, student: &User, extra_attempts: u16) -> DomainResult<Self> {
        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "receive extra attempts",
            });
        }

        if !assignment.is_issued_to(student) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup);
        }

        ensure_total_attempts(assignment.max_attempts(), extra_attempts)?;

        Ok(Self {
            assignment_id: assignment.id(),
            student_id: student.id(),
            extra_attempts,
        })
    }

    pub fn grant_extra_attempts(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        granted_by: &User,
        amount: u16,
    ) -> DomainResult<()> {
        if self.assignment_id != assignment.id() {
            return Err(DomainError::StudentProgressDoesNotBelongToAssignment);
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

        let extra_attempts =
            self.extra_attempts
                .checked_add(amount)
                .ok_or(DomainError::AttemptLimitExceeded {
                    value: u32::from(self.extra_attempts) + u32::from(amount),
                    max: MAX_ATTEMPTS,
                })?;
        ensure_total_attempts(assignment.max_attempts(), extra_attempts)?;
        self.extra_attempts = extra_attempts;

        Ok(())
    }

    pub fn assignment_id(&self) -> AssignmentId {
        self.assignment_id
    }

    pub fn student_id(&self) -> UserId {
        self.student_id
    }

    pub fn extra_attempts(&self) -> u16 {
        self.extra_attempts
    }
}

fn ensure_total_attempts(max_attempts: u16, extra_attempts: u16) -> DomainResult<()> {
    let total = u32::from(max_attempts) + u32::from(extra_attempts);
    if total > u32::from(MAX_ATTEMPTS) {
        return Err(DomainError::AttemptLimitExceeded {
            value: total,
            max: MAX_ATTEMPTS,
        });
    }

    Ok(())
}
