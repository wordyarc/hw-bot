use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::{PgConnection, PgPool};
use thiserror::Error;
use uuid::Uuid;

use crate::db::{DbError, academic, course_invites};
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

    #[error("bootstrap: {0}")]
    Database(#[from] DbError),
}

impl From<sqlx::Error> for BootstrapError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.into())
    }
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
    #[serde(default)]
    sandbox: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CourseSeed {
    subject_name: String,
    student_group_codes: Vec<String>,
    teacher_telegram_user_ids: Vec<i64>,
}

struct ValidatedSeed {
    staff: Vec<User>,
    student_groups: Vec<StudentGroup>,
    subjects: Vec<ValidatedSubject>,
    courses: Vec<ValidatedCourse>,
}

struct ValidatedSubject {
    subject: Subject,
    sandbox: bool,
}

struct ValidatedCourse {
    subject_name: String,
    student_group_codes: Vec<String>,
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
        subject_ids.insert(subject.subject.name().as_str(), id);
    }

    let mut course_ids = HashSet::with_capacity(seed.courses.len());
    for course in &seed.courses {
        let subject_id = subject_ids[course.subject_name.as_str()];
        let student_group_ids: Vec<Uuid> = course
            .student_group_codes
            .iter()
            .map(|code| group_ids[code.as_str()])
            .collect();
        let course_id = ensure_course(&mut transaction, subject_id, &student_group_ids).await?;
        if !course_ids.insert(course_id) {
            return Err(invalid(format!(
                "two courses of subject {} resolve to one stored course",
                course.subject_name
            )));
        }

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
        let sandbox = subject.sandbox;
        let subject = Subject::new(SubjectId::new(), subject.name)?;
        if !subject_names.insert(subject.name().as_str().to_owned()) {
            return Err(invalid("subject names must be unique"));
        }
        subjects.push(ValidatedSubject { subject, sandbox });
    }

    let mut courses = Vec::with_capacity(seed.courses.len());
    let mut course_keys = HashSet::with_capacity(seed.courses.len());
    for course in seed.courses {
        let subject_name = course.subject_name.trim().to_owned();
        if course.student_group_codes.is_empty() {
            return Err(invalid("every course must have at least one student group"));
        }
        let student_group_codes: Vec<String> = course
            .student_group_codes
            .iter()
            .map(|code| code.trim().to_owned())
            .collect();
        if !subject_names.contains(&subject_name)
            || student_group_codes
                .iter()
                .any(|code| !group_codes.contains(code))
        {
            return Err(invalid(
                "every course must reference a subject and student groups from this seed",
            ));
        }
        let unique_codes: HashSet<&String> = student_group_codes.iter().collect();
        if unique_codes.len() != student_group_codes.len() {
            return Err(invalid(
                "a student group must not be repeated within a course",
            ));
        }
        for code in &student_group_codes {
            if !course_keys.insert((subject_name.clone(), code.clone())) {
                return Err(invalid("subject and student group pairs must be unique"));
            }
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
            student_group_codes,
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

    Ok(row.id)
}

async fn ensure_subject(
    connection: &mut PgConnection,
    seed: &ValidatedSubject,
) -> Result<Uuid, BootstrapError> {
    let subject = &seed.subject;
    sqlx::query!(
        r#"
        insert into subjects (id, name, sandbox)
        values ($1, $2, $3)
        on conflict (name) do nothing
        "#,
        subject.id().as_uuid(),
        subject.name().as_str(),
        seed.sandbox,
    )
    .execute(&mut *connection)
    .await?;

    let row = sqlx::query!(
        r#"
        select id, sandbox
        from subjects
        where name = $1
        for share
        "#,
        subject.name().as_str(),
    )
    .fetch_one(connection)
    .await?;

    if row.sandbox != seed.sandbox {
        return Err(invalid(format!(
            "existing subject {} conflicts with requested data",
            subject.name().as_str()
        )));
    }

    Ok(row.id)
}

async fn ensure_course(
    connection: &mut PgConnection,
    subject_id: Uuid,
    student_group_ids: &[Uuid],
) -> Result<Uuid, BootstrapError> {
    let mut stored = sqlx::query_scalar!(
        r#"
        select teaching_course_id
        from teaching_course_groups
        where subject_id = $1 and student_group_id = any($2)
        for share
        "#,
        subject_id,
        student_group_ids,
    )
    .fetch_all(&mut *connection)
    .await?;
    stored.sort_unstable();
    stored.dedup();

    let course_id = match stored[..] {
        [] => {
            let course_id = TeachingCourseId::new();
            sqlx::query!(
                r#"
                insert into teaching_courses (id, subject_id)
                values ($1, $2)
                "#,
                course_id.as_uuid(),
                subject_id,
            )
            .execute(&mut *connection)
            .await?;
            course_id
        }
        [course_id] => TeachingCourseId::from_uuid(course_id),
        _ => {
            return Err(invalid(
                "a course of the seed joins student groups of different stored courses",
            ));
        }
    };

    for &student_group_id in student_group_ids {
        academic::add_teaching_course_group(
            &mut *connection,
            course_id,
            StudentGroupId::from_uuid(student_group_id),
        )
        .await?;
    }
    course_invites::ensure(connection, course_id).await?;

    Ok(course_id.as_uuid())
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
    .fetch_one(&mut *connection)
    .await?;
    if has_course_without_teacher {
        return Err(invalid(
            "every teaching course must have at least one teacher",
        ));
    }

    let has_course_without_group = sqlx::query_scalar!(
        r#"
        select exists (
            select 1
            from teaching_courses course
            where not exists (
                select 1
                from teaching_course_groups course_group
                where course_group.teaching_course_id = course.id
            )
        ) as "exists!"
        "#,
    )
    .fetch_one(connection)
    .await?;
    if has_course_without_group {
        return Err(invalid(
            "every teaching course must have at least one student group",
        ));
    }

    Ok(())
}

fn invalid(message: impl Into<String>) -> BootstrapError {
    BootstrapError::InvalidData(message.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::test_support::{bootstrap_seed, closed_pool};

    #[track_caller]
    fn rejects(value: serde_json::Value, reason: &str) {
        let result = validate_seed(Seed::deserialize(value).unwrap());
        assert!(
            matches!(&result, Err(BootstrapError::InvalidData(message)) if message == reason),
            "expected {reason}, got {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn blank_unicode_name_is_rejected_before_accessing_the_database() {
        let pool = closed_pool().await;
        let mut seed = bootstrap_seed();
        seed["admin"]["display_name"] = json!("\u{a0}");
        assert!(matches!(
            run_bootstrap(&pool, &seed).await,
            Err(BootstrapError::Domain(DomainError::EmptyField { .. }))
        ));
    }

    #[test]
    fn staff_identities_must_be_unique() {
        let mut seed = bootstrap_seed();
        seed["teachers"][0]["telegram_user_id"] = seed["admin"]["telegram_user_id"].clone();
        rejects(seed, "staff telegram_user_id values must be unique");
    }

    #[test]
    fn group_codes_are_unique_after_trimming() {
        let mut seed = bootstrap_seed();
        let mut duplicate = seed["student_groups"][0].clone();
        duplicate["code"] = json!(" b9122-02.03.21 ");
        seed["student_groups"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        rejects(seed, "student group codes must be unique");
    }

    #[test]
    fn subject_names_are_unique_after_trimming() {
        let mut seed = bootstrap_seed();
        seed["subjects"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": " Backend Development "}));
        rejects(seed, "subject names must be unique");
    }

    #[test]
    fn course_pairs_are_unique_after_trimming() {
        let mut seed = bootstrap_seed();
        let mut duplicate = seed["courses"][0].clone();
        duplicate["subject_name"] = json!(" Backend Development ");
        duplicate["student_group_codes"] = json!([" b9122-02.03.21 "]);
        seed["courses"].as_array_mut().unwrap().push(duplicate);
        rejects(seed, "subject and student group pairs must be unique");
    }

    #[test]
    fn courses_reference_groups_and_subjects_from_the_same_seed() {
        for (field, value) in [
            ("subject_name", json!("unknown")),
            ("student_group_codes", json!(["unknown"])),
            ("student_group_codes", json!(["b9122-02.03.21", "unknown"])),
        ] {
            let mut seed = bootstrap_seed();
            seed["courses"][0][field] = value;
            rejects(
                seed,
                "every course must reference a subject and student groups from this seed",
            );
        }
    }

    #[test]
    fn a_course_requires_at_least_one_student_group() {
        let mut seed = bootstrap_seed();
        seed["courses"][0]["student_group_codes"] = json!([]);
        rejects(seed, "every course must have at least one student group");
    }

    #[test]
    fn a_course_cannot_repeat_a_student_group() {
        let mut seed = bootstrap_seed();
        seed["courses"][0]["student_group_codes"] = json!(["b9122-02.03.21", " b9122-02.03.21 "]);
        rejects(seed, "a student group must not be repeated within a course");
    }

    #[test]
    fn a_course_of_the_former_format_is_rejected_before_the_database() {
        let mut seed = bootstrap_seed();
        let course = seed["courses"][0].as_object_mut().unwrap();
        course.remove("student_group_codes");
        course.insert("student_group_code".to_owned(), json!("b9122-02.03.21"));

        assert!(Seed::deserialize(seed).is_err());
    }

    #[test]
    fn course_teachers_reference_staff_from_the_same_seed() {
        let mut seed = bootstrap_seed();
        seed["courses"][0]["teacher_telegram_user_ids"] = json!([999]);
        rejects(
            seed,
            "every course teacher must reference the admin or a teacher from this seed",
        );
    }

    #[test]
    fn a_course_requires_at_least_one_teacher() {
        let mut seed = bootstrap_seed();
        seed["courses"][0]["teacher_telegram_user_ids"] = json!([]);
        rejects(seed, "every course must have at least one teacher");
    }

    #[test]
    fn a_course_cannot_repeat_a_teacher() {
        let mut seed = bootstrap_seed();
        seed["courses"][0]["teacher_telegram_user_ids"] = json!([100000002, 100000002]);
        rejects(seed, "a teacher must not be repeated within a course");
    }

    #[test]
    fn the_seed_of_the_default_config_is_valid() {
        let seed: Seed = serde_json::from_str(include_str!("../../config/bootstrap.json"))
            .expect("the default seed matches the seed format");
        validate_seed(seed).expect("the default seed is valid");
    }

    #[test]
    fn the_deployment_example_is_a_valid_seed_with_a_sandbox_course() {
        let seed: Seed = serde_json::from_str(include_str!("../../dist/examples/bootstrap.json"))
            .expect("the example matches the seed format");
        let seed = validate_seed(seed).expect("the example is a valid seed");

        let sandbox = seed
            .subjects
            .iter()
            .find(|subject| subject.sandbox)
            .expect("the example shows a sandbox subject");
        let sandbox_course = seed
            .courses
            .iter()
            .find(|course| course.subject_name == sandbox.subject.name().as_str())
            .expect("the example shows a sandbox course");
        assert_eq!(
            sandbox_course.student_group_codes.len(),
            seed.student_groups.len(),
            "the sandbox course spans every group"
        );
        assert!(
            seed.courses
                .iter()
                .any(|course| course.student_group_codes.len() > 1),
            "the example shows a course of several groups"
        );
    }

    #[test]
    fn required_academic_collections_cannot_be_empty() {
        for collection in ["student_groups", "subjects", "courses"] {
            let mut seed = bootstrap_seed();
            seed[collection] = json!([]);
            rejects(
                seed,
                "student_groups, subjects and courses must not be empty",
            );
        }
    }
}
