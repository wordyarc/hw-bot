#![allow(dead_code)]

use chrono::{Duration, Utc};
use hw_bot::db;
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
        db::academic::insert_student_group(pool, &group).await?;

        let subject = Subject::new(SubjectId::new(), "Backend development")?;
        db::academic::insert_subject(pool, &subject).await?;

        let teacher = User::new(
            UserId::new(),
            TelegramUserId::new(1000)?,
            Role::Teacher,
            "Teacher",
            Utc::now(),
        )?;
        db::users::insert(pool, &teacher).await?;

        let student = User::new_student(
            UserId::new(),
            TelegramUserId::new(1001)?,
            "Student",
            &group,
            Utc::now(),
        )?;
        db::users::insert(pool, &student).await?;

        let course = TeachingCourse::new(
            TeachingCourseId::new(),
            &subject,
            &group,
            &teacher,
            Utc::now(),
        )?;
        let mut tx = pool.begin().await?;
        db::academic::insert_teaching_course(&mut tx, &course).await?;
        tx.commit().await?;

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
        db::assignments::insert(pool, &assignment).await?;

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
        db::users::insert(pool, &student).await?;

        Ok(student)
    }

    /// Преподаватель без привязки к курсу фикстуры: нужен, чтобы проверять, что чужие сдачи ему
    /// не видны и не подчиняются.
    pub async fn insert_teacher(
        &self,
        pool: &PgPool,
        telegram_id: i64,
        display_name: &str,
    ) -> anyhow::Result<User> {
        let teacher = User::new(
            UserId::new(),
            TelegramUserId::new(telegram_id)?,
            Role::Teacher,
            display_name,
            Utc::now(),
        )?;
        db::users::insert(pool, &teacher).await?;

        Ok(teacher)
    }

    pub async fn insert_admin(&self, pool: &PgPool, telegram_id: i64) -> anyhow::Result<User> {
        let admin = User::new(
            UserId::new(),
            TelegramUserId::new(telegram_id)?,
            Role::Admin,
            "Admin",
            Utc::now(),
        )?;
        db::users::insert(pool, &admin).await?;

        Ok(admin)
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
        db::academic::insert_student_group(pool, &other_group).await?;

        let student = User::new_student(
            UserId::new(),
            TelegramUserId::new(telegram_id)?,
            "Outsider",
            &other_group,
            Utc::now(),
        )?;
        db::users::insert(pool, &student).await?;

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
        db::assignments::insert(pool, &assignment).await?;

        Ok(assignment)
    }
}
