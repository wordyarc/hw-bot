use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use thiserror::Error;

use crate::db::{DbError, academic, users};
use crate::domain::{
    DomainError, EducationLevel, Role, StudentGroup, StudentGroupId, StudyProgram, Subject,
    SubjectId, TeachingCourse, TeachingCourseId, TelegramUserId, User, UserId,
};

const BOOTSTRAP_LOCK: &str = "hw-bot/bootstrap/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapOutcome {
    Applied,
    AlreadyPopulated,
}

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("bootstrap: failed to read {}: {source}", path.display())]
    File {
        path: PathBuf,
        source: std::io::Error,
    },

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

pub async fn run_bootstrap_file(
    pool: &PgPool,
    file: &Path,
) -> Result<BootstrapOutcome, BootstrapError> {
    if users::any_exist(pool).await? {
        return Ok(BootstrapOutcome::AlreadyPopulated);
    }

    let data = tokio::fs::read_to_string(file)
        .await
        .map_err(|source| BootstrapError::File {
            path: file.to_owned(),
            source,
        })?;
    run_bootstrap(pool, &serde_json::from_str(&data)?).await
}

pub async fn run_bootstrap(
    pool: &PgPool,
    value: &serde_json::Value,
) -> Result<BootstrapOutcome, BootstrapError> {
    let seed = validate_seed(Seed::deserialize(value)?)?;
    let mut transaction = pool.begin().await?;

    sqlx::query!(
        "select pg_advisory_xact_lock(hashtextextended($1, 0))",
        BOOTSTRAP_LOCK,
    )
    .execute(&mut *transaction)
    .await?;

    if users::any_exist(&mut *transaction).await? {
        return Ok(BootstrapOutcome::AlreadyPopulated);
    }

    let mut staff = HashMap::with_capacity(seed.staff.len());
    for user in &seed.staff {
        users::insert(&mut *transaction, user).await?;
        staff.insert(user.telegram_user_id(), user);
    }

    let mut groups = HashMap::with_capacity(seed.student_groups.len());
    for group in &seed.student_groups {
        academic::insert_student_group(&mut *transaction, group).await?;
        groups.insert(group.code().as_str(), group);
    }

    let mut subjects = HashMap::with_capacity(seed.subjects.len());
    for ValidatedSubject { subject, sandbox } in &seed.subjects {
        academic::insert_subject(&mut *transaction, subject, *sandbox).await?;
        subjects.insert(subject.name().as_str(), subject);
    }

    let now = Utc::now();
    for course in &seed.courses {
        let course_groups: Vec<&StudentGroup> = course
            .student_group_codes
            .iter()
            .map(|code| groups[code.as_str()])
            .collect();
        let teachers: Vec<&User> = course
            .teacher_telegram_user_ids
            .iter()
            .map(|id| staff[id])
            .collect();
        let course = TeachingCourse::new(
            TeachingCourseId::new(),
            subjects[course.subject_name.as_str()],
            &course_groups,
            &teachers,
            now,
        )?;
        academic::insert_teaching_course(&mut transaction, &course).await?;
    }

    transaction.commit().await?;
    Ok(BootstrapOutcome::Applied)
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
