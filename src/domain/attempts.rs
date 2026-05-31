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

        if student.student_group_id() != Some(assignment.student_group_id()) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup);
        }

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

        self.extra_attempts = self.extra_attempts.saturating_add(amount);

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
