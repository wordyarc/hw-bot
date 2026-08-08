use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::domain::enums::str_enum;
use crate::domain::{
    DomainError, DomainResult, NonEmptyString, StudentGroupId, SubjectId, TeachingCourseId, User,
    UserId,
};

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum EducationLevel {
        Bachelor => "bachelor",
        Master => "master",
        Specialist => "specialist",
        Postgraduate => "postgraduate",
    }
}

impl EducationLevel {
    pub fn max_study_year(self) -> u8 {
        match self {
            Self::Bachelor => 4,
            Self::Master => 2,
            Self::Specialist => 6,
            Self::Postgraduate => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StudyProgram {
    education_level: EducationLevel,
    year: u8,
}

impl StudyProgram {
    pub fn new(education_level: EducationLevel, year: u8) -> DomainResult<Self> {
        let max = education_level.max_study_year();

        if year == 0 || year > max {
            return Err(DomainError::InvalidStudyYear {
                education_level: education_level.as_str(),
                value: year,
                max,
            });
        }

        Ok(Self {
            education_level,
            year,
        })
    }

    pub fn education_level(self) -> EducationLevel {
        self.education_level
    }

    pub fn year(self) -> u8 {
        self.year
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StudentGroup {
    id: StudentGroupId,
    code: NonEmptyString,
    program: StudyProgram,
}

impl StudentGroup {
    pub fn new(
        id: StudentGroupId,
        code: impl Into<String>,
        program: StudyProgram,
    ) -> DomainResult<Self> {
        Ok(Self {
            id,
            code: NonEmptyString::new("student_group.code", code)?,
            program,
        })
    }

    pub fn id(&self) -> StudentGroupId {
        self.id
    }

    pub fn code(&self) -> &NonEmptyString {
        &self.code
    }

    pub fn program(&self) -> StudyProgram {
        self.program
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subject {
    id: SubjectId,
    name: NonEmptyString,
}

impl Subject {
    pub fn new(id: SubjectId, name: impl Into<String>) -> DomainResult<Self> {
        Ok(Self {
            id,
            name: NonEmptyString::new("subject.name", name)?,
        })
    }

    pub fn id(&self) -> SubjectId {
        self.id
    }

    pub fn name(&self) -> &NonEmptyString {
        &self.name
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TeachingCourse {
    id: TeachingCourseId,
    subject_id: SubjectId,
    student_group_id: StudentGroupId,
    teacher_ids: HashSet<UserId>,
    created_at: DateTime<Utc>,
}

impl TeachingCourse {
    pub(crate) fn restore(
        id: TeachingCourseId,
        subject_id: SubjectId,
        student_group_id: StudentGroupId,
        teacher_ids: HashSet<UserId>,
        created_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if teacher_ids.is_empty() {
            return Err(DomainError::TeachingCourseHasNoTeachers);
        }

        Ok(Self {
            id,
            subject_id,
            student_group_id,
            teacher_ids,
            created_at,
        })
    }

    pub fn new(
        id: TeachingCourseId,
        subject: &Subject,
        student_group: &StudentGroup,
        teacher: &User,
        created_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !teacher.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "create teaching course",
            });
        }

        Self::restore(
            id,
            subject.id(),
            student_group.id(),
            HashSet::from([teacher.id()]),
            created_at,
        )
    }

    pub fn add_teacher(&mut self, actor: &User, teacher: &User) -> DomainResult<()> {
        if !self.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied {
                action: "add course teacher",
            });
        }

        if !teacher.role().can_manage_assignments() {
            return Err(DomainError::PermissionDenied {
                action: "teach course",
            });
        }

        self.teacher_ids.insert(teacher.id());

        Ok(())
    }

    pub fn can_be_managed_by(&self, actor: &User) -> bool {
        actor.role().can_manage_all_teaching_courses()
            || (actor.role().can_manage_assignments() && self.teacher_ids.contains(&actor.id()))
    }

    pub fn id(&self) -> TeachingCourseId {
        self.id
    }

    pub fn subject_id(&self) -> SubjectId {
        self.subject_id
    }

    pub fn student_group_id(&self) -> StudentGroupId {
        self.student_group_id
    }

    pub fn teacher_ids(&self) -> &HashSet<UserId> {
        &self.teacher_ids
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::t0;

    #[test]
    fn study_program_validates_year_by_education_level() {
        assert_eq!(
            StudyProgram::new(EducationLevel::Master, 3).unwrap_err(),
            DomainError::InvalidStudyYear {
                education_level: "master",
                value: 3,
                max: 2,
            }
        );
    }

    #[test]
    fn restore_rejects_teaching_course_without_teachers() {
        assert_eq!(
            TeachingCourse::restore(
                TeachingCourseId::new(),
                SubjectId::new(),
                StudentGroupId::new(),
                HashSet::new(),
                t0(),
            )
            .unwrap_err(),
            DomainError::TeachingCourseHasNoTeachers
        );
    }

    #[test]
    fn study_program_accepts_valid_year() {
        let program = StudyProgram::new(EducationLevel::Bachelor, 4).unwrap();

        assert_eq!(program.education_level(), EducationLevel::Bachelor);
        assert_eq!(program.year(), 4);
    }
}
