#![allow(dead_code)]

use chrono::{Duration, Utc};
use hw_bot::domain::{
    Assignment, AssignmentId, EducationLevel, Role, StudentGroup, StudentGroupId, StudyProgram,
    Subject, SubjectId, TeachingCourse, TeachingCourseId, TelegramUserId, User, UserId,
};
use sqlx::PgPool;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

const POSTGRES_PORT: u16 = 5432;

pub async fn postgres_pool() -> anyhow::Result<(ContainerAsync<GenericImage>, PgPool)> {
    let postgres = GenericImage::new("postgres", "18.4-bookworm")
        .with_exposed_port(POSTGRES_PORT.tcp())
        .with_wait_for(WaitFor::message_on_stderr(
            "database system is ready to accept connections",
        ))
        .with_env_var("POSTGRES_USER", "postgres")
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        .with_env_var("POSTGRES_DB", "hw_bot_test")
        .start()
        .await?;

    let host = postgres.get_host().await?;
    let port = postgres.get_host_port_ipv4(POSTGRES_PORT.tcp()).await?;
    let database_url = format!("postgres://postgres:postgres@{host}:{port}/hw_bot_test");

    Ok((postgres, PgPool::connect(&database_url).await?))
}

pub struct Fixture {
    pub group: StudentGroup,
    pub subject: Subject,
    pub teacher: User,
    pub student: User,
    pub course: TeachingCourse,
    pub assignment: Assignment,
}

impl Fixture {
    pub async fn insert(pool: &PgPool) -> anyhow::Result<Self> {
        let group = StudentGroup::new(
            StudentGroupId::new(),
            "B9122-02.03.21",
            StudyProgram::new(EducationLevel::Bachelor, 1)?,
        )?;
        insert_group(pool, &group).await?;

        let subject = Subject::new(SubjectId::new(), "Backend development")?;
        insert_subject(pool, &subject).await?;

        let teacher = User::new(
            UserId::new(),
            TelegramUserId::new(1000)?,
            Role::Teacher,
            "Teacher",
            Utc::now(),
        )?;
        insert_user(pool, &teacher).await?;

        let student = User::new_student(
            UserId::new(),
            TelegramUserId::new(1001)?,
            "Student",
            &group,
            Utc::now(),
        )?;
        insert_user(pool, &student).await?;

        let course = TeachingCourse::new(
            TeachingCourseId::new(),
            &subject,
            &group,
            &teacher,
            Utc::now(),
        )?;
        insert_course(pool, &course).await?;
        insert_course_teacher(pool, &course, &teacher).await?;

        let created_at = Utc::now();
        let mut assignment = Assignment::new(
            AssignmentId::new(),
            &course,
            &teacher,
            "Rust ownership",
            "Send a pull request link.",
            2,
            created_at,
            created_at + Duration::days(7),
        )?;
        assignment.publish(&course, &teacher, created_at + Duration::seconds(1))?;
        insert_assignment(pool, &assignment).await?;

        Ok(Self {
            group,
            subject,
            teacher,
            student,
            course,
            assignment,
        })
    }

    pub async fn insert_student(
        &self,
        pool: &PgPool,
        telegram_id: i64,
        display_name: &str,
    ) -> anyhow::Result<User> {
        let student = User::new_student(
            UserId::new(),
            TelegramUserId::new(telegram_id)?,
            display_name,
            &self.group,
            Utc::now(),
        )?;
        insert_user(pool, &student).await?;

        Ok(student)
    }

    pub async fn insert_outsider_student(
        &self,
        pool: &PgPool,
        telegram_id: i64,
    ) -> anyhow::Result<User> {
        let other_group = StudentGroup::new(
            StudentGroupId::new(),
            "B9123-02.03.21",
            StudyProgram::new(EducationLevel::Bachelor, 2)?,
        )?;
        insert_group(pool, &other_group).await?;

        let student = User::new_student(
            UserId::new(),
            TelegramUserId::new(telegram_id)?,
            "Outsider",
            &other_group,
            Utc::now(),
        )?;
        insert_user(pool, &student).await?;

        Ok(student)
    }

    pub async fn insert_draft_assignment(&self, pool: &PgPool) -> anyhow::Result<Assignment> {
        let created_at = Utc::now();
        let assignment = Assignment::new(
            AssignmentId::new(),
            &self.course,
            &self.teacher,
            "Rust lifetimes",
            "Not ready yet.",
            1,
            created_at,
            created_at + Duration::days(7),
        )?;
        insert_assignment(pool, &assignment).await?;

        Ok(assignment)
    }
}

pub async fn insert_group(pool: &PgPool, group: &StudentGroup) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into student_groups (id, code, education_level, study_year)
        values ($1, $2, $3, $4)
        "#,
    )
    .bind(group.id().as_uuid())
    .bind(group.code().as_str())
    .bind(group.program().education_level().as_str())
    .bind(i16::from(group.program().year()))
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_subject(pool: &PgPool, subject: &Subject) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into subjects (id, name)
        values ($1, $2)
        "#,
    )
    .bind(subject.id().as_uuid())
    .bind(subject.name().as_str())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_user(pool: &PgPool, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into users (
            id,
            telegram_user_id,
            role,
            display_name,
            student_group_id,
            registered_at
        )
        values ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user.id().as_uuid())
    .bind(user.telegram_user_id().as_i64())
    .bind(user.role().as_db_str())
    .bind(user.display_name().as_str())
    .bind(user.student_group_id().map(StudentGroupId::as_uuid))
    .bind(user.registered_at())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_course(pool: &PgPool, course: &TeachingCourse) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into teaching_courses (
            id,
            subject_id,
            student_group_id,
            created_at
        )
        values ($1, $2, $3, $4)
        "#,
    )
    .bind(course.id().as_uuid())
    .bind(course.subject_id().as_uuid())
    .bind(course.student_group_id().as_uuid())
    .bind(course.created_at())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_course_teacher(
    pool: &PgPool,
    course: &TeachingCourse,
    teacher: &User,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into teaching_course_teachers (teaching_course_id, teacher_id)
        values ($1, $2)
        "#,
    )
    .bind(course.id().as_uuid())
    .bind(teacher.id().as_uuid())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_assignment(pool: &PgPool, assignment: &Assignment) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        insert into assignments (
            id,
            teaching_course_id,
            subject_id,
            student_group_id,
            title,
            body,
            max_attempts,
            status,
            created_by,
            created_at,
            deadline_at,
            published_at
        )
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(assignment.id().as_uuid())
    .bind(assignment.teaching_course_id().as_uuid())
    .bind(assignment.subject_id().as_uuid())
    .bind(assignment.student_group_id().as_uuid())
    .bind(assignment.title().as_str())
    .bind(assignment.body().as_str())
    .bind(i16::try_from(assignment.max_attempts())?)
    .bind(assignment.status().as_db_str())
    .bind(assignment.created_by().as_uuid())
    .bind(assignment.created_at())
    .bind(assignment.deadline_at())
    .bind(assignment.published_at())
    .execute(pool)
    .await?;

    Ok(())
}
