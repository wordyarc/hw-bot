use chrono::{Duration, SubsecRound, Utc};
use hw_bot::db;
use hw_bot::domain::{Assignment, StudentGroup, Subject, TeachingCourse, User};
use hw_bot::test_support::{
    AssignmentBuilder, GROUP_CODE, SUBJECT_NAME, assignment, named_student, named_teacher,
    student_group, subject, teaching_course,
};
use sqlx::PgPool;

/// HTTP-хендлер выставляет `submitted_at = Utc::now()`. Для таких сдач нужен дедлайн
/// от текущего времени: срок, отсчитанный от `t0()`, к запуску теста уже мог пройти.
pub fn wall_clock_assignment<'a>(
    course: &'a TeachingCourse,
    author: &'a User,
) -> AssignmentBuilder<'a> {
    assignment(course, author)
        .created_at(Utc::now().trunc_subsecs(6) - Duration::hours(1))
        .deadline_in(Duration::days(30))
}

pub struct Fixture {
    pub group: StudentGroup,
    pub subject: Subject,
    pub teacher: User,
    pub student: User,
    pub course: TeachingCourse,
    pub assignment: Assignment,
}

pub struct CourseFixture {
    pub group: StudentGroup,
    pub teacher: User,
    pub course: TeachingCourse,
}

impl Fixture {
    pub async fn insert(pool: &PgPool) -> Self {
        let group = student_group(GROUP_CODE);
        db::academic::insert_student_group(pool, &group)
            .await
            .expect("insert student group");

        let subject = subject(SUBJECT_NAME);
        db::academic::insert_subject(pool, &subject)
            .await
            .expect("insert subject");

        let teacher = named_teacher(1000, "Teacher");
        insert_user(pool, &teacher).await;

        let student = named_student(1001, "Student", &group);
        insert_user(pool, &student).await;

        let course = teaching_course(&subject, &group, &teacher);
        insert_course(pool, &course).await;

        let assignment = wall_clock_assignment(&course, &teacher).published().build();
        db::assignments::insert(pool, &assignment)
            .await
            .expect("insert assignment");

        Self {
            group,
            subject,
            teacher,
            student,
            course,
            assignment,
        }
    }
}

pub async fn insert_user(pool: &PgPool, user: &User) {
    db::users::insert(pool, user).await.expect("insert user");
}

pub async fn insert_group(pool: &PgPool, group: &StudentGroup) {
    db::academic::insert_student_group(pool, group)
        .await
        .expect("insert student group");
}

pub async fn insert_course(pool: &PgPool, course: &TeachingCourse) {
    let mut tx = pool.begin().await.expect("begin transaction");
    db::academic::insert_teaching_course(&mut tx, course)
        .await
        .expect("insert teaching course");
    tx.commit().await.expect("commit transaction");
}
