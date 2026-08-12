use chrono::{DateTime, Utc};

use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, AssignmentId, DomainError, DomainResult, NonEmptyString,
    StudentGroupId, SubjectId, TeachingCourse, TeachingCourseId, USER_TEXT_MAX_LEN, User, UserId,
};
use crate::string_enum::str_enum;

pub const MAX_ATTEMPTS: u16 = i16::MAX as u16;

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AssignmentStatus {
        Draft => "draft",
        Published => "published",
        Archived => "archived",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    id: AssignmentId,
    teaching_course_id: TeachingCourseId,
    subject_id: SubjectId,
    student_group_id: StudentGroupId,
    title: NonEmptyString,
    body: NonEmptyString,
    max_attempts: u16,
    status: AssignmentStatus,
    created_by: UserId,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

/// Поля задания россыпью: `restore` собирает сущность из них по именам, чтобы соседние
/// `DateTime<Utc>` нельзя было переставить местами незаметно для компилятора.
pub(crate) struct AssignmentParts {
    pub id: AssignmentId,
    pub teaching_course_id: TeachingCourseId,
    pub subject_id: SubjectId,
    pub student_group_id: StudentGroupId,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub status: AssignmentStatus,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

impl Assignment {
    pub(crate) fn restore(parts: AssignmentParts) -> DomainResult<Self> {
        if parts.max_attempts == 0 || parts.max_attempts > MAX_ATTEMPTS {
            return Err(DomainError::InvalidAttemptLimit {
                value: parts.max_attempts,
            });
        }

        if parts.deadline_at <= parts.created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }

        // An archived assignment may or may not have been published before, so it is the only
        // status that accepts both shapes.
        match parts.status {
            AssignmentStatus::Draft if parts.published_at.is_some() => {
                return Err(DomainError::InvalidAssignmentPublishedState);
            }
            AssignmentStatus::Published if parts.published_at.is_none() => {
                return Err(DomainError::InvalidAssignmentPublishedState);
            }
            _ => {}
        }

        Ok(Self {
            id: parts.id,
            teaching_course_id: parts.teaching_course_id,
            subject_id: parts.subject_id,
            student_group_id: parts.student_group_id,
            title: NonEmptyString::new_bounded(
                "assignment.title",
                parts.title,
                ASSIGNMENT_TITLE_MAX_LEN,
            )?,
            body: NonEmptyString::new_bounded("assignment.body", parts.body, USER_TEXT_MAX_LEN)?,
            max_attempts: parts.max_attempts,
            status: parts.status,
            created_by: parts.created_by,
            created_at: parts.created_at,
            deadline_at: parts.deadline_at,
            published_at: parts.published_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
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

        Self::restore(AssignmentParts {
            id,
            teaching_course_id: teaching_course.id(),
            subject_id: teaching_course.subject_id(),
            student_group_id: teaching_course.student_group_id(),
            title: title.into(),
            body: body.into(),
            max_attempts,
            status: AssignmentStatus::Draft,
            created_by: created_by.id(),
            created_at,
            deadline_at,
            published_at: None,
        })
    }

    pub fn publish(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        published_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "publish assignment")?;

        if self.status == AssignmentStatus::Published {
            return Err(DomainError::AssignmentStatusTransitionNotAllowed {
                from: self.status,
                to: AssignmentStatus::Published,
            });
        }

        self.status = AssignmentStatus::Published;
        self.published_at = Some(published_at);

        Ok(())
    }

    pub fn archive(&mut self, teaching_course: &TeachingCourse, actor: &User) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "archive assignment")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentStatusTransitionNotAllowed {
                from: self.status,
                to: AssignmentStatus::Archived,
            });
        }

        self.status = AssignmentStatus::Archived;

        Ok(())
    }

    pub fn change_deadline(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        new_deadline_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "change assignment deadline")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentIsArchived);
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

    pub fn ensure_belongs_to_course(&self, teaching_course: &TeachingCourse) -> DomainResult<()> {
        if self.teaching_course_id != teaching_course.id() {
            return Err(DomainError::AssignmentDoesNotBelongToTeachingCourse);
        }

        Ok(())
    }

    pub(crate) fn ensure_managed_by(
        &self,
        teaching_course: &TeachingCourse,
        actor: &User,
        action: &'static str,
    ) -> DomainResult<()> {
        self.ensure_belongs_to_course(teaching_course)?;

        if !teaching_course.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied { action });
        }

        Ok(())
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

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::test_support::t0;

    fn restore(
        status: AssignmentStatus,
        max_attempts: u16,
        published_at: Option<DateTime<Utc>>,
    ) -> DomainResult<Assignment> {
        let created_at = t0();

        Assignment::restore(AssignmentParts {
            id: AssignmentId::new(),
            teaching_course_id: TeachingCourseId::new(),
            subject_id: SubjectId::new(),
            student_group_id: StudentGroupId::new(),
            title: "Rust ownership".to_owned(),
            body: "Send a pull request link.".to_owned(),
            max_attempts,
            status,
            created_by: UserId::new(),
            created_at,
            deadline_at: created_at + Duration::days(7),
            published_at,
        })
    }

    #[test]
    fn restores_published_assignment() {
        let published_at = t0();
        let assignment = restore(AssignmentStatus::Published, 2, Some(published_at)).unwrap();

        assert!(assignment.is_published());
        assert_eq!(assignment.published_at(), Some(published_at));
        assert_eq!(assignment.max_attempts(), 2);
    }

    #[test]
    fn restores_archived_assignment_regardless_of_published_at() {
        assert!(restore(AssignmentStatus::Archived, 1, None).is_ok());
        assert!(restore(AssignmentStatus::Archived, 1, Some(t0())).is_ok());
    }

    #[test]
    fn restore_rejects_zero_max_attempts() {
        assert_eq!(
            restore(AssignmentStatus::Draft, 0, None).unwrap_err(),
            DomainError::InvalidAttemptLimit { value: 0 }
        );
    }

    #[test]
    fn restore_rejects_max_attempts_that_do_not_fit_storage() {
        assert_eq!(
            restore(AssignmentStatus::Draft, MAX_ATTEMPTS + 1, None).unwrap_err(),
            DomainError::InvalidAttemptLimit {
                value: MAX_ATTEMPTS + 1,
            }
        );
    }

    #[test]
    fn restore_rejects_published_assignment_without_published_at() {
        assert_eq!(
            restore(AssignmentStatus::Published, 1, None).unwrap_err(),
            DomainError::InvalidAssignmentPublishedState
        );
    }

    #[test]
    fn restore_rejects_draft_assignment_with_published_at() {
        assert_eq!(
            restore(AssignmentStatus::Draft, 1, Some(t0())).unwrap_err(),
            DomainError::InvalidAssignmentPublishedState
        );
    }
}
