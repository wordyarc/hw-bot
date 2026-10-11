use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};

use crate::domain::{
    DomainError, DomainResult, NonEmptyString, STUDENT_GROUP_CODE_MAX_LEN, SUBJECT_NAME_MAX_LEN,
    StudentGroupId, SubjectId, TeachingCourseId, User, UserId,
};
use crate::string_enum::str_enum;

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
            code: NonEmptyString::new_bounded(
                "student_group.code",
                code,
                STUDENT_GROUP_CODE_MAX_LEN,
            )?,
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
            name: NonEmptyString::new_bounded("subject.name", name, SUBJECT_NAME_MAX_LEN)?,
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
    group_ids: BTreeSet<StudentGroupId>,
    teacher_ids: HashSet<UserId>,
    created_at: DateTime<Utc>,
}

impl TeachingCourse {
    pub(crate) fn restore(
        id: TeachingCourseId,
        subject_id: SubjectId,
        group_ids: BTreeSet<StudentGroupId>,
        teacher_ids: HashSet<UserId>,
        created_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if group_ids.is_empty() {
            return Err(DomainError::TeachingCourseHasNoGroups);
        }

        if teacher_ids.is_empty() {
            return Err(DomainError::TeachingCourseHasNoTeachers);
        }

        Ok(Self {
            id,
            subject_id,
            group_ids,
            teacher_ids,
            created_at,
        })
    }

    pub fn new(
        id: TeachingCourseId,
        subject: &Subject,
        groups: &[&StudentGroup],
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
            groups.iter().map(|group| group.id()).collect(),
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

    pub fn group_ids(&self) -> &BTreeSet<StudentGroupId> {
        &self.group_ids
    }

    pub fn includes_group(&self, group_id: StudentGroupId) -> bool {
        self.group_ids.contains(&group_id)
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
    use crate::test_support::{student_group, subject, t0, teacher, teaching_course};

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
                BTreeSet::from([StudentGroupId::new()]),
                HashSet::new(),
                t0(),
            )
            .unwrap_err(),
            DomainError::TeachingCourseHasNoTeachers
        );
    }

    #[test]
    fn restore_rejects_teaching_course_without_groups() {
        assert_eq!(
            TeachingCourse::restore(
                TeachingCourseId::new(),
                SubjectId::new(),
                BTreeSet::new(),
                HashSet::from([UserId::new()]),
                t0(),
            )
            .unwrap_err(),
            DomainError::TeachingCourseHasNoGroups
        );
    }

    #[test]
    fn a_course_includes_only_its_own_groups() {
        let teacher = teacher(1);
        let (first, second, foreign) = (
            student_group("first"),
            student_group("second"),
            student_group("foreign"),
        );
        let course = teaching_course(&subject("Backend"), &[&first, &second], &teacher);

        assert!(course.includes_group(first.id()));
        assert!(course.includes_group(second.id()));
        assert!(!course.includes_group(foreign.id()));
        assert_eq!(course.group_ids().len(), 2);
    }

    #[test]
    fn study_program_accepts_valid_year() {
        let program = StudyProgram::new(EducationLevel::Bachelor, 4).unwrap();

        assert_eq!(program.education_level(), EducationLevel::Bachelor);
        assert_eq!(program.year(), 4);
    }
}
