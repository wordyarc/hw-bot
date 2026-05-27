use chrono::{DateTime, Utc};

use crate::domain::{
    AssignmentId, DomainError, DomainResult, NonEmptyString, StudentGroupId, SubjectId,
    TeachingCourse, TeachingCourseId, User, UserId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssignmentStatus {
    Draft,
    Published,
    Archived,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    id: AssignmentId,
    teaching_course_id: TeachingCourseId,
    subject_id: SubjectId,
    student_group_id: StudentGroupId,
    teacher_id: UserId,
    title: NonEmptyString,
    body: NonEmptyString,
    max_attempts: u16,
    status: AssignmentStatus,
    created_by: UserId,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

impl Assignment {
    pub fn new(
        id: AssignmentId,
        teaching_course: &TeachingCourse,
        created_by: &User,
        title: impl Into<String>,
        body: impl Into<String>,
        max_attempts: u16,
        created_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !teaching_course.can_be_managed_by(created_by) {
            return Err(DomainError::PermissionDenied {
                action: "create assignment",
            });
        }

        if max_attempts == 0 {
            return Err(DomainError::InvalidAttemptLimit {
                value: max_attempts,
            });
        }

        if deadline_at <= created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }

        Ok(Self {
            id,
            teaching_course_id: teaching_course.id(),
            subject_id: teaching_course.subject_id(),
            student_group_id: teaching_course.student_group_id(),
            teacher_id: teaching_course.teacher_id(),
            title: NonEmptyString::new("assignment.title", title)?,
            body: NonEmptyString::new("assignment.body", body)?,
            max_attempts,
            status: AssignmentStatus::Draft,
            created_by: created_by.id(),
            created_at,
            deadline_at,
            published_at: None,
        })
    }

    pub fn publish(&mut self, actor: &User, published_at: DateTime<Utc>) -> DomainResult<()> {
        if !self.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied {
                action: "publish assignment",
            });
        }

        self.status = AssignmentStatus::Published;
        self.published_at = Some(published_at);

        Ok(())
    }

    pub fn archive(&mut self, actor: &User) -> DomainResult<()> {
        if !self.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied {
                action: "archive assignment",
            });
        }

        self.status = AssignmentStatus::Archived;

        Ok(())
    }

    pub fn change_deadline(
        &mut self,
        actor: &User,
        new_deadline_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        if !self.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied {
                action: "change assignment deadline",
            });
        }

        if new_deadline_at <= self.created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }

        self.deadline_at = new_deadline_at;

        Ok(())
    }

    pub fn id(&self) -> AssignmentId {
        self.id
    }

    pub fn teaching_course_id(&self) -> TeachingCourseId {
        self.teaching_course_id
    }

    pub fn subject_id(&self) -> SubjectId {
        self.subject_id
    }

    pub fn student_group_id(&self) -> StudentGroupId {
        self.student_group_id
    }

    pub fn teacher_id(&self) -> UserId {
        self.teacher_id
    }

    pub fn can_be_managed_by(&self, actor: &User) -> bool {
        actor.role().can_manage_all_teaching_courses()
            || (actor.role().can_manage_assignments() && actor.id() == self.teacher_id)
    }

    pub fn title(&self) -> &NonEmptyString {
        &self.title
    }

    pub fn body(&self) -> &NonEmptyString {
        &self.body
    }

    pub fn max_attempts(&self) -> u16 {
        self.max_attempts
    }

    pub fn status(&self) -> AssignmentStatus {
        self.status
    }

    pub fn created_by(&self) -> UserId {
        self.created_by
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn deadline_at(&self) -> DateTime<Utc> {
        self.deadline_at
    }

    pub fn published_at(&self) -> Option<DateTime<Utc>> {
        self.published_at
    }

    pub fn is_published(&self) -> bool {
        self.status == AssignmentStatus::Published
    }
}
