use chrono::{DateTime, Utc};

use crate::domain::{
    DomainError, DomainResult, NonEmptyString, StudentGroupId, SubjectId, TeachingCourseId, User,
    UserId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EducationLevel {
    Bachelor,
    Master,
    Specialist,
    Postgraduate,
}

impl EducationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bachelor => "bachelor",
            Self::Master => "master",
            Self::Specialist => "specialist",
            Self::Postgraduate => "postgraduate",
        }
    }

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
    teacher_id: UserId,
    created_at: DateTime<Utc>,
}

impl TeachingCourse {
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

        Ok(Self {
            id,
            subject_id: subject.id(),
            student_group_id: student_group.id(),
            teacher_id: teacher.id(),
            created_at,
        })
    }

    pub fn can_be_managed_by(&self, actor: &User) -> bool {
        actor.role().can_manage_all_teaching_courses()
            || (actor.role().can_manage_assignments() && actor.id() == self.teacher_id)
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

    pub fn teacher_id(&self) -> UserId {
        self.teacher_id
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}
