use chrono::{DateTime, Duration, Utc};

pub mod telegram;

use crate::domain::{
    Assignment, AssignmentId, DomainResult, EducationLevel, Grading, LatePolicy, NewAssignment,
    Role, StudentGroup, StudentGroupId, StudyProgram, Subject, SubjectId, Submission,
    SubmissionHistory, SubmissionId, TeachingCourse, TeachingCourseId, TelegramUserId, User,
    UserId,
};

#[cfg(test)]
pub(crate) async fn closed_pool() -> sqlx::PgPool {
    let pool = sqlx::PgPool::connect_lazy("postgres://postgres:postgres@127.0.0.1:1/unused")
        .expect("valid test database URL");
    pool.close().await;
    pool
}

pub fn bootstrap_seed() -> serde_json::Value {
    serde_json::json!({
        "admin": {
            "telegram_user_id": 100000001,
            "display_name": "First Admin"
        },
        "teachers": [{
            "telegram_user_id": 100000002,
            "display_name": "Course Teacher"
        }],
        "student_groups": [{
            "code": "b9122-02.03.21",
            "education_level": "bachelor",
            "study_year": 1
        }],
        "subjects": [{"name": "Backend Development"}],
        "courses": [{
            "subject_name": "Backend Development",
            "student_group_code": "b9122-02.03.21",
            "teacher_telegram_user_ids": [100000002]
        }]
    })
}

pub const GROUP_CODE: &str = "B9122-02.03.21";
pub const OTHER_GROUP_CODE: &str = "M9241-09.04.03";
pub const SUBJECT_NAME: &str = "Backend development";
pub const ASSIGNMENT_TITLE: &str = "Rust ownership";
pub const ASSIGNMENT_BODY: &str = "Send a pull request link.";
pub const PULL_REQUEST_URL: &str = "https://github.com/course/hw/pull/1";

pub fn t0() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).expect("2026-01-01T00:00:00Z is a valid timestamp")
}

pub fn student_group(code: &str) -> StudentGroup {
    student_group_in(code, EducationLevel::Bachelor, 1)
}

pub fn student_group_in(code: &str, level: EducationLevel, year: u8) -> StudentGroup {
    StudentGroup::new(
        StudentGroupId::new(),
        code,
        StudyProgram::new(level, year).expect("valid study program"),
    )
    .expect("valid student group")
}

pub fn subject(name: &str) -> Subject {
    Subject::new(SubjectId::new(), name).expect("valid subject")
}

pub fn teacher(telegram_id: i64) -> User {
    named_teacher(telegram_id, &format!("Teacher {telegram_id}"))
}

pub fn admin(telegram_id: i64) -> User {
    named_admin(telegram_id, &format!("Admin {telegram_id}"))
}

pub fn student(telegram_id: i64, group: &StudentGroup) -> User {
    named_student(telegram_id, &format!("Student {telegram_id}"), group)
}

pub fn named_teacher(telegram_id: i64, display_name: &str) -> User {
    staff(telegram_id, Role::Teacher, display_name)
}

pub fn named_admin(telegram_id: i64, display_name: &str) -> User {
    staff(telegram_id, Role::Admin, display_name)
}

pub fn named_student(telegram_id: i64, display_name: &str, group: &StudentGroup) -> User {
    User::new_student(
        UserId::new(),
        telegram_user_id(telegram_id),
        display_name,
        group,
        t0(),
    )
    .expect("valid student")
}

fn staff(telegram_id: i64, role: Role, display_name: &str) -> User {
    User::new(
        UserId::new(),
        telegram_user_id(telegram_id),
        role,
        display_name,
        t0(),
    )
    .expect("valid staff user")
}

fn telegram_user_id(value: i64) -> TelegramUserId {
    TelegramUserId::new(value).expect("positive telegram user id")
}

pub fn teaching_course(subject: &Subject, group: &StudentGroup, teacher: &User) -> TeachingCourse {
    TeachingCourse::new(TeachingCourseId::new(), subject, group, teacher, t0())
        .expect("valid teaching course")
}

pub fn course_for(group: &StudentGroup, teacher: &User) -> TeachingCourse {
    teaching_course(&subject(SUBJECT_NAME), group, teacher)
}

pub fn assignment<'a>(course: &'a TeachingCourse, author: &'a User) -> AssignmentBuilder<'a> {
    AssignmentBuilder {
        course,
        author,
        title: ASSIGNMENT_TITLE.to_owned(),
        body: ASSIGNMENT_BODY.to_owned(),
        max_attempts: 2,
        grading: Grading::PassFail,
        late_policy: None,
        created_at: t0(),
        deadline_at: None,
        deadline_offset: Duration::days(7),
        publish: false,
    }
}

pub struct AssignmentBuilder<'a> {
    course: &'a TeachingCourse,
    author: &'a User,
    title: String,
    body: String,
    max_attempts: u16,
    grading: Grading,
    late_policy: Option<LatePolicy>,
    created_at: DateTime<Utc>,
    deadline_at: Option<DateTime<Utc>>,
    deadline_offset: Duration,
    publish: bool,
}

impl<'a> AssignmentBuilder<'a> {
    pub fn title(mut self, value: impl Into<String>) -> Self {
        self.title = value.into();
        self
    }

    pub fn body(mut self, value: impl Into<String>) -> Self {
        self.body = value.into();
        self
    }

    pub fn max_attempts(mut self, value: u16) -> Self {
        self.max_attempts = value;
        self
    }

    pub fn created_at(mut self, value: DateTime<Utc>) -> Self {
        self.created_at = value;
        self
    }

    #[track_caller]
    pub fn scored(mut self, max_score: u16) -> Self {
        self.grading = Grading::scored(max_score).expect("valid max score");
        self
    }

    pub fn grading(mut self, value: Grading) -> Self {
        self.grading = value;
        self
    }

    #[track_caller]
    pub fn late_policy(mut self, period_days: u16, penalty: u16, max_periods: u16) -> Self {
        self.late_policy =
            Some(LatePolicy::new(period_days, penalty, max_periods).expect("valid late policy"));
        self
    }

    /// Смещение дедлайна позволяет вызвать `.created_at()` после `.deadline_in()`:
    /// срок пересчитается относительно новой даты создания.
    pub fn deadline_in(mut self, offset: Duration) -> Self {
        self.deadline_offset = offset;
        self.deadline_at = None;
        self
    }

    pub fn deadline_at(mut self, value: DateTime<Utc>) -> Self {
        self.deadline_at = Some(value);
        self
    }

    pub fn published(mut self) -> Self {
        self.publish = true;
        self
    }

    #[track_caller]
    pub fn build(self) -> Assignment {
        self.try_build().expect("valid assignment")
    }

    pub fn try_build(self) -> DomainResult<Assignment> {
        let deadline_at = self
            .deadline_at
            .unwrap_or(self.created_at + self.deadline_offset);

        let mut assignment = Assignment::new(
            AssignmentId::new(),
            self.course,
            self.author,
            NewAssignment {
                title: self.title,
                body: self.body,
                max_attempts: self.max_attempts,
                grading: self.grading,
                late_policy: self.late_policy,
                created_at: self.created_at,
                deadline_at,
            },
        )?;

        if self.publish {
            assignment.publish(self.course, self.author, self.created_at)?;
        }

        Ok(assignment)
    }
}

pub fn submission<'a>(assignment: &'a Assignment, student: &'a User) -> SubmissionBuilder<'a> {
    SubmissionBuilder {
        assignment,
        student,
        history: SubmissionHistory::default(),
        content: PULL_REQUEST_URL.to_owned(),
        submitted_at: assignment.created_at() + Duration::hours(1),
    }
}

pub struct SubmissionBuilder<'a> {
    assignment: &'a Assignment,
    student: &'a User,
    history: SubmissionHistory,
    content: String,
    submitted_at: DateTime<Utc>,
}

impl<'a> SubmissionBuilder<'a> {
    pub fn history(mut self, value: SubmissionHistory) -> Self {
        self.history = value;
        self
    }

    pub fn content(mut self, value: impl Into<String>) -> Self {
        self.content = value.into();
        self
    }

    pub fn submitted_at(mut self, value: DateTime<Utc>) -> Self {
        self.submitted_at = value;
        self
    }

    #[track_caller]
    pub fn build(self) -> Submission {
        self.try_build().expect("valid submission")
    }

    pub fn try_build(self) -> DomainResult<Submission> {
        Submission::submit(
            SubmissionId::new(),
            self.assignment,
            self.student,
            self.history,
            self.content,
            self.submitted_at,
        )
    }
}

pub fn history() -> HistoryBuilder {
    HistoryBuilder::default()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HistoryBuilder {
    used_attempts: u16,
    extra_attempts: u16,
    has_accepted: bool,
    has_pending: bool,
}

impl HistoryBuilder {
    pub fn used(mut self, value: u16) -> Self {
        self.used_attempts = value;
        self
    }

    pub fn extra(mut self, value: u16) -> Self {
        self.extra_attempts = value;
        self
    }

    pub fn accepted(mut self) -> Self {
        self.has_accepted = true;
        self
    }

    pub fn pending(mut self) -> Self {
        self.has_pending = true;
        self
    }

    pub fn build(self) -> SubmissionHistory {
        SubmissionHistory::new(
            self.used_attempts,
            self.extra_attempts,
            self.has_accepted,
            self.has_pending,
        )
    }
}
