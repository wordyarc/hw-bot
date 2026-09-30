use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{PgConnection, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::db::{DbError, group_invites};
use crate::domain::{
    DomainError, EducationLevel, Role, StudentGroup, StudentGroupId, StudyProgram, Subject,
    SubjectId, TeachingCourseId, TelegramUserId, User, UserId,
};

const BOOTSTRAP_LOCK: &str = "hw-bot/bootstrap/v1";

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("bootstrap: invalid JSON data: {0}")]
    Json(#[from] serde_json::Error),

    #[error("bootstrap: invalid domain data: {0}")]
    Domain(#[from] DomainError),

    #[error("bootstrap: {0}")]
    InvalidData(String),

    #[error("bootstrap: database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("bootstrap: database error: {0}")]
    Db(#[from] DbError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Seed {
    admin: StaffSeed,
    teachers: Vec<StaffSeed>,
    student_groups: Vec<StudentGroupSeed>,
    subjects: Vec<SubjectSeed>,
    courses: Vec<CourseSeed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StaffSeed {
    telegram_user_id: i64,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StudentGroupSeed {
    code: String,
    education_level: String,
    study_year: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectSeed {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CourseSeed {
    subject_name: String,
    student_group_code: String,
    teacher_telegram_user_ids: Vec<i64>,
}

struct ValidatedSeed {
    staff: Vec<User>,
    student_groups: Vec<StudentGroup>,
    subjects: Vec<Subject>,
    courses: Vec<ValidatedCourse>,
}

struct ValidatedCourse {
    subject_name: String,
    student_group_code: String,
    teacher_telegram_user_ids: Vec<TelegramUserId>,
}

pub async fn run_bootstrap(pool: &PgPool, value: &serde_json::Value) -> Result<(), BootstrapError> {
    let seed = validate_seed(Seed::deserialize(value)?)?;
    let mut transaction = pool.begin().await?;

    sqlx::query!(
        "select pg_advisory_xact_lock(hashtextextended($1, 0))",
        BOOTSTRAP_LOCK,
    )
    .execute(&mut *transaction)
    .await?;

    let mut staff_ids = HashMap::with_capacity(seed.staff.len());
    for staff in &seed.staff {
        let id = ensure_staff(&mut transaction, staff).await?;
        staff_ids.insert(staff.telegram_user_id(), id);
    }

    let mut group_ids = HashMap::with_capacity(seed.student_groups.len());
    for group in &seed.student_groups {
        let id = ensure_student_group(&mut transaction, group).await?;
        group_ids.insert(group.code().as_str(), id);
    }

    let mut subject_ids = HashMap::with_capacity(seed.subjects.len());
    for subject in &seed.subjects {
        let id = ensure_subject(&mut transaction, subject).await?;
        subject_ids.insert(subject.name().as_str(), id);
    }

    for course in &seed.courses {
        let subject_id = subject_ids[course.subject_name.as_str()];
        let student_group_id = group_ids[course.student_group_code.as_str()];
        let course_id = ensure_course(&mut transaction, subject_id, student_group_id).await?;

        for telegram_user_id in &course.teacher_telegram_user_ids {
            ensure_course_teacher(&mut transaction, course_id, staff_ids[telegram_user_id]).await?;
        }
    }

    validate_stored_courses(&mut transaction).await?;
    transaction.commit().await?;
    Ok(())
}

fn validate_seed(seed: Seed) -> Result<ValidatedSeed, BootstrapError> {
    if seed.student_groups.is_empty() || seed.subjects.is_empty() || seed.courses.is_empty() {
        return Err(invalid(
            "student_groups, subjects and courses must not be empty",
        ));
    }

    let now = Utc::now();
    let mut staff = Vec::with_capacity(seed.teachers.len() + 1);
    staff.push(make_staff(seed.admin, Role::Admin, now)?);
    for teacher in seed.teachers {
        staff.push(make_staff(teacher, Role::Teacher, now)?);
    }

    let mut staff_telegram_ids = HashSet::with_capacity(staff.len());
    for user in &staff {
        if !staff_telegram_ids.insert(user.telegram_user_id()) {
            return Err(invalid("staff telegram_user_id values must be unique"));
        }
    }

    let mut student_groups = Vec::with_capacity(seed.student_groups.len());
    let mut group_codes = HashSet::with_capacity(seed.student_groups.len());
    for group in seed.student_groups {
        let education_level = EducationLevel::parse_str(group.education_level.trim())
            .ok_or_else(|| invalid("student group education_level is invalid"))?;
        let group = StudentGroup::new(
            StudentGroupId::new(),
            group.code,
            StudyProgram::new(education_level, group.study_year)?,
        )?;
        if !group_codes.insert(group.code().as_str().to_owned()) {
            return Err(invalid("student group codes must be unique"));
        }
        student_groups.push(group);
    }

    let mut subjects = Vec::with_capacity(seed.subjects.len());
    let mut subject_names = HashSet::with_capacity(seed.subjects.len());
    for subject in seed.subjects {
        let subject = Subject::new(SubjectId::new(), subject.name)?;
        if !subject_names.insert(subject.name().as_str().to_owned()) {
            return Err(invalid("subject names must be unique"));
        }
        subjects.push(subject);
    }

    let mut courses = Vec::with_capacity(seed.courses.len());
    let mut course_keys = HashSet::with_capacity(seed.courses.len());
    for course in seed.courses {
        let subject_name = course.subject_name.trim().to_owned();
        let student_group_code = course.student_group_code.trim().to_owned();
        if !subject_names.contains(&subject_name) || !group_codes.contains(&student_group_code) {
            return Err(invalid(
                "every course must reference a subject and student group from this seed",
            ));
        }
        if !course_keys.insert((subject_name.clone(), student_group_code.clone())) {
            return Err(invalid("subject and student group pairs must be unique"));
        }
        if course.teacher_telegram_user_ids.is_empty() {
            return Err(invalid("every course must have at least one teacher"));
        }

        let mut teacher_ids = Vec::with_capacity(course.teacher_telegram_user_ids.len());
        let mut unique_teacher_ids = HashSet::with_capacity(course.teacher_telegram_user_ids.len());
        for value in course.teacher_telegram_user_ids {
            let telegram_user_id = TelegramUserId::new(value)?;
            if !staff_telegram_ids.contains(&telegram_user_id) {
                return Err(invalid(
                    "every course teacher must reference the admin or a teacher from this seed",
                ));
            }
            if !unique_teacher_ids.insert(telegram_user_id) {
                return Err(invalid("a teacher must not be repeated within a course"));
            }
            teacher_ids.push(telegram_user_id);
        }

        courses.push(ValidatedCourse {
            subject_name,
            student_group_code,
            teacher_telegram_user_ids: teacher_ids,
        });
    }

    Ok(ValidatedSeed {
        staff,
        student_groups,
        subjects,
        courses,
    })
}

fn make_staff(
    seed: StaffSeed,
    role: Role,
    registered_at: DateTime<Utc>,
) -> Result<User, DomainError> {
    User::new(
        UserId::new(),
        TelegramUserId::new(seed.telegram_user_id)?,
        role,
        seed.display_name,
        registered_at,
    )
}

async fn ensure_staff(connection: &mut PgConnection, user: &User) -> Result<Uuid, BootstrapError> {
    sqlx::query!(
        r#"
        insert into users (id, telegram_user_id, role, display_name, registered_at)
        values ($1, $2, $3, $4, $5)
        on conflict (telegram_user_id) do nothing
        "#,
        user.id().as_uuid(),
        user.telegram_user_id().as_i64(),
        user.role().as_str(),
        user.display_name().as_str(),
        user.registered_at(),
    )
    .execute(&mut *connection)
    .await?;

    let row = sqlx::query!(
        r#"
        select id, role, display_name, student_group_id
        from users
        where telegram_user_id = $1
        for share
        "#,
        user.telegram_user_id().as_i64(),
    )
    .fetch_one(connection)
    .await?;

    if row.role != user.role().as_str()
        || row.display_name != user.display_name().as_str()
        || row.student_group_id.is_some()
    {
        return Err(invalid(format!(
            "existing user {} conflicts with requested staff data",
            user.telegram_user_id().as_i64()
        )));
    }

    Ok(row.id)
}

async fn ensure_student_group(
    connection: &mut PgConnection,
    group: &StudentGroup,
) -> Result<Uuid, BootstrapError> {
    sqlx::query!(
        r#"
        insert into student_groups (id, code, education_level, study_year)
        values ($1, $2, $3, $4)
        on conflict (code) do nothing
        "#,
        group.id().as_uuid(),
        group.code().as_str(),
        group.program().education_level().as_str(),
        i16::from(group.program().year()),
    )
    .execute(&mut *connection)
    .await?;

    let row = sqlx::query!(
        r#"
        select id, education_level, study_year
        from student_groups
        where code = $1
        for share
        "#,
        group.code().as_str(),
    )
    .fetch_one(&mut *connection)
    .await?;

    if row.education_level != group.program().education_level().as_str()
        || row.study_year != i16::from(group.program().year())
    {
        return Err(invalid(format!(
            "existing student group {} conflicts with requested data",
            group.code().as_str()
        )));
    }

    group_invites::ensure(connection, StudentGroupId::from_uuid(row.id)).await?;

    Ok(row.id)
}

async fn ensure_subject(
    connection: &mut PgConnection,
    subject: &Subject,
) -> Result<Uuid, BootstrapError> {
    sqlx::query!(
        r#"
        insert into subjects (id, name)
        values ($1, $2)
        on conflict (name) do nothing
        "#,
        subject.id().as_uuid(),
        subject.name().as_str(),
    )
    .execute(&mut *connection)
    .await?;

    Ok(sqlx::query_scalar!(
        r#"
        select id
        from subjects
        where name = $1
        for share
        "#,
        subject.name().as_str(),
    )
    .fetch_one(connection)
    .await?)
}

async fn ensure_course(
    connection: &mut PgConnection,
    subject_id: Uuid,
    student_group_id: Uuid,
) -> Result<Uuid, BootstrapError> {
    sqlx::query!(
        r#"
        insert into teaching_courses (id, subject_id, student_group_id)
        values ($1, $2, $3)
        on conflict (subject_id, student_group_id) do nothing
        "#,
        TeachingCourseId::new().as_uuid(),
        subject_id,
        student_group_id,
    )
    .execute(&mut *connection)
    .await?;

    Ok(sqlx::query_scalar!(
        r#"
        select id
        from teaching_courses
        where subject_id = $1 and student_group_id = $2
        for share
        "#,
        subject_id,
        student_group_id,
    )
    .fetch_one(connection)
    .await?)
}

async fn ensure_course_teacher(
    connection: &mut PgConnection,
    teaching_course_id: Uuid,
    teacher_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        insert into teaching_course_teachers (teaching_course_id, teacher_id)
        values ($1, $2)
        on conflict (teaching_course_id, teacher_id) do nothing
        "#,
        teaching_course_id,
        teacher_id,
    )
    .execute(connection)
    .await?;
    Ok(())
}

async fn validate_stored_courses(connection: &mut PgConnection) -> Result<(), BootstrapError> {
    let has_student_teacher = sqlx::query_scalar!(
        r#"
        select exists (
            select 1
            from teaching_course_teachers course_teacher
            join users teacher on teacher.id = course_teacher.teacher_id
            where teacher.role = 'student'
        ) as "exists!"
        "#,
    )
    .fetch_one(&mut *connection)
    .await?;
    if has_student_teacher {
        return Err(invalid("a course teacher assignment references a student"));
    }

    let has_course_without_teacher = sqlx::query_scalar!(
        r#"
        select exists (
            select 1
            from teaching_courses course
            where not exists (
                select 1
                from teaching_course_teachers course_teacher
                join users teacher on teacher.id = course_teacher.teacher_id
                where course_teacher.teaching_course_id = course.id
                  and teacher.role in ('teacher', 'admin')
            )
        ) as "exists!"
        "#,
    )
    .fetch_one(connection)
    .await?;
    if has_course_without_teacher {
        return Err(invalid(
            "every teaching course must have at least one teacher",
        ));
    }

    Ok(())
}

fn invalid(message: impl Into<String>) -> BootstrapError {
    BootstrapError::InvalidData(message.into())
}
