use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration as StdDuration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::request::Builder;
use chrono::{DateTime, Duration, SubsecRound, Utc};
use hw_bot::api::{self, ACTOR_HEADER, AppState};
use hw_bot::app::{Actor, ApplicationContainer, ConfigureMakeup, FilePolicy, MakeupSettings};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, CourseMakeup, DeductedFrom, EducationLevel, Grading, MakeupTerms, ReviewScore,
    StudentGroup, StudentGroupId, Subject, TeachingCourse, TeachingCourseId, User,
};
use hw_bot::health::HealthState;
use hw_bot::invites::{CourseInviteCodec, InviteSigningSecret};
use hw_bot::test_support::{
    assignment as assignment_of, named_admin, named_student, named_teacher, student_group_in,
    subject, teaching_course,
};
use serde_json::{Value, json};
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

mod database;
mod fixture;
mod response;
mod response_gate;
pub mod telegram;
pub mod telegram_scenario;

pub use database::{
    fresh_database, migrated_database, saturated_database, wait_for_blocked_connection,
};
pub use fixture::{CourseFixture, Fixture, insert_assignment, wall_clock_assignment};
pub use response::TestResponse;
pub use response_gate::ResponseGate;

use fixture::{insert_course, insert_group, insert_user};

const INVITE_SIGNING_SECRET: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

pub fn invite_codec() -> CourseInviteCodec {
    CourseInviteCodec::new(InviteSigningSecret::from_base64url(INVITE_SIGNING_SECRET).unwrap())
}

pub struct TestApp {
    pub pool: PgPool,
    pub fixture: Fixture,
    pub application: ApplicationContainer,
    pub invites: CourseInviteCodec,
    router: Router,
    next_telegram_id: AtomicI64,
}

impl TestApp {
    pub async fn start() -> Self {
        Self::start_with_http_timeout(StdDuration::from_secs(10)).await
    }

    pub async fn start_with_http_timeout(request_timeout: StdDuration) -> Self {
        Self::over(migrated_database().await, request_timeout).await
    }

    /// В пуле одно соединение. Заняв его, тест воспроизводит ошибку исчерпания пула.
    pub async fn start_saturated() -> Self {
        Self::over(saturated_database().await, StdDuration::from_secs(10)).await
    }

    async fn over(pool: PgPool, request_timeout: StdDuration) -> Self {
        let fixture = Fixture::insert(&pool).await;
        let application = ApplicationContainer::new(pool.clone(), test_file_policy());
        let invites = invite_codec();
        let router = api::router(
            AppState::new(
                pool.clone(),
                application.clone(),
                invites.clone(),
                HealthState::new(false, false),
            ),
            request_timeout,
        );

        Self {
            pool,
            fixture,
            application,
            invites,
            router,
            // Не пересекается с ID, записанными в тестах литералами.
            next_telegram_id: AtomicI64::new(5000),
        }
    }

    pub fn assignment(&self) -> Uuid {
        self.fixture.assignment.id().as_uuid()
    }

    pub fn course(&self) -> Uuid {
        self.fixture.course.id().as_uuid()
    }

    pub fn teacher(&self) -> i64 {
        self.fixture.teacher.telegram_user_id().as_i64()
    }

    pub fn teacher_actor(&self) -> Actor {
        Actor::new(self.fixture.teacher.id())
    }

    pub fn student(&self) -> i64 {
        self.fixture.student.telegram_user_id().as_i64()
    }

    pub fn student_actor(&self) -> Actor {
        Actor::new(self.fixture.student.id())
    }

    pub fn next_telegram_id(&self) -> i64 {
        self.next_telegram_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn get(&self, path: &str) -> TestResponse {
        self.send(empty_request(Request::get(path))).await
    }

    pub async fn get_as(&self, path: &str, actor: i64) -> TestResponse {
        self.send(empty_request(
            Request::get(path).header(ACTOR_HEADER, actor),
        ))
        .await
    }

    pub async fn student_assignments(&self, actor: i64) -> TestResponse {
        self.get_as("/me/assignments", actor).await
    }

    pub async fn archived_student_assignments(&self, actor: i64) -> TestResponse {
        self.get_as("/me/assignments/archive", actor).await
    }

    pub async fn student_assignment(&self, assignment: Uuid, actor: i64) -> TestResponse {
        self.get_as(&format!("/me/assignments/{assignment}"), actor)
            .await
    }

    pub async fn teacher_courses(&self, actor: i64) -> TestResponse {
        self.get_as("/me/courses", actor).await
    }

    pub async fn course_assignments(&self, course: Uuid, actor: i64) -> TestResponse {
        self.get_as(&format!("/teaching-courses/{course}/assignments"), actor)
            .await
    }

    pub async fn review_queue_size(&self, actor: i64) -> TestResponse {
        self.get_as("/me/review-queue", actor).await
    }

    pub async fn post_as(&self, path: &str, actor: i64, body: Value) -> TestResponse {
        self.send(json_request(
            Request::post(path).header(ACTOR_HEADER, actor),
            body,
        ))
        .await
    }

    pub async fn post_empty_as(&self, path: &str, actor: i64) -> TestResponse {
        self.send(empty_request(
            Request::post(path).header(ACTOR_HEADER, actor),
        ))
        .await
    }

    async fn send(&self, request: Request<Body>) -> TestResponse {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router responds");
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("read response body");

        TestResponse::new(parts.status, parts.headers, bytes)
    }

    pub async fn submit(&self, assignment: Uuid, telegram_id: i64, content: &str) -> TestResponse {
        self.submit_raw(assignment, telegram_id, json!({ "content": content }))
            .await
    }

    pub async fn submit_raw(
        &self,
        assignment: Uuid,
        telegram_id: i64,
        body: Value,
    ) -> TestResponse {
        self.post_as(
            &format!("/assignments/{assignment}/submissions"),
            telegram_id,
            body,
        )
        .await
    }

    pub async fn queued_submission(&self, telegram_id: i64, content: &str) -> Uuid {
        let body = self.submit(self.assignment(), telegram_id, content).await;

        submission_id(&body.created())
    }

    pub async fn create_assignment(&self, course: Uuid, telegram_id: i64) -> TestResponse {
        self.create_assignment_raw(
            course,
            telegram_id,
            json!({
                "title": "Rust lifetimes",
                "body": "Send a pull request link.",
                "max_attempts": 2,
                "deadline_at": Utc::now() + Duration::days(7),
            }),
        )
        .await
    }

    pub async fn create_assignment_raw(
        &self,
        course: Uuid,
        telegram_id: i64,
        body: Value,
    ) -> TestResponse {
        self.post_as(
            &format!("/teaching-courses/{course}/assignments"),
            telegram_id,
            body,
        )
        .await
    }

    pub async fn drafted_assignment(&self, telegram_id: i64) -> Uuid {
        let body = self.create_assignment(self.course(), telegram_id).await;

        assignment_id(&body.created())
    }

    pub async fn publish_assignment(&self, assignment: Uuid, telegram_id: i64) -> TestResponse {
        self.post_empty_as(&format!("/assignments/{assignment}/publish"), telegram_id)
            .await
    }

    pub async fn archive_assignment(&self, assignment: Uuid, telegram_id: i64) -> TestResponse {
        self.post_empty_as(&format!("/assignments/{assignment}/archive"), telegram_id)
            .await
    }

    pub async fn change_deadline(
        &self,
        assignment: Uuid,
        telegram_id: i64,
        deadline_at: DateTime<Utc>,
    ) -> TestResponse {
        self.post_as(
            &format!("/assignments/{assignment}/deadline"),
            telegram_id,
            json!({ "deadline_at": deadline_at }),
        )
        .await
    }

    pub async fn grant_extra_attempts(
        &self,
        assignment: Uuid,
        student: Uuid,
        telegram_id: i64,
        amount: u16,
    ) -> TestResponse {
        self.post_as(
            &format!("/assignments/{assignment}/students/{student}/extra-attempts"),
            telegram_id,
            json!({ "amount": amount }),
        )
        .await
    }

    pub async fn take_next(&self, telegram_id: i64) -> TestResponse {
        self.post_empty_as("/submissions/next", telegram_id).await
    }

    pub async fn review(
        &self,
        submission: Uuid,
        telegram_id: i64,
        decision: &str,
        comment: &str,
    ) -> TestResponse {
        self.review_raw(
            submission,
            telegram_id,
            json!({ "decision": decision, "comment": comment }),
        )
        .await
    }

    pub async fn review_with_score(
        &self,
        submission: Uuid,
        telegram_id: i64,
        decision: &str,
        score: ReviewScore,
        comment: &str,
    ) -> TestResponse {
        self.review_raw(
            submission,
            telegram_id,
            json!({
                "decision": decision,
                "score": score.value,
                "max_score": score.max_score,
                "comment": comment,
            }),
        )
        .await
    }

    pub async fn review_raw(
        &self,
        submission: Uuid,
        telegram_id: i64,
        body: Value,
    ) -> TestResponse {
        self.post_as(
            &format!("/submissions/{submission}/review"),
            telegram_id,
            body,
        )
        .await
    }

    pub async fn change_grading(
        &self,
        assignment: Uuid,
        telegram_id: i64,
        max_score: Option<u16>,
    ) -> TestResponse {
        self.post_as(
            &format!("/assignments/{assignment}/grading"),
            telegram_id,
            json!({ "max_score": max_score }),
        )
        .await
    }

    /// Токен подписывается с текущей generation из БД. После ротации нужен новый вызов.
    pub async fn invite_token(&self, course_id: TeachingCourseId) -> String {
        let generation = db::course_invites::find_generation(&self.pool, course_id)
            .await
            .expect("read invite generation")
            .expect("course has an invite");

        self.invites.encode(course_id, generation)
    }

    pub async fn course_invite_token(&self) -> String {
        self.invite_token(self.fixture.course.id()).await
    }

    pub async fn register_student(
        &self,
        telegram_id: i64,
        display_name: &str,
        invite_token: &str,
    ) -> TestResponse {
        self.post_as(
            "/students",
            telegram_id,
            json!({
                "display_name": display_name,
                "invite_token": invite_token,
            }),
        )
        .await
    }

    pub async fn register_student_in(
        &self,
        telegram_id: i64,
        display_name: &str,
        invite_token: &str,
        group_id: StudentGroupId,
    ) -> TestResponse {
        self.post_as(
            "/students",
            telegram_id,
            json!({
                "display_name": display_name,
                "invite_token": invite_token,
                "student_group_id": group_id.as_uuid(),
            }),
        )
        .await
    }

    pub async fn register_staff(
        &self,
        actor: i64,
        telegram_id: i64,
        display_name: &str,
        role: &str,
    ) -> TestResponse {
        self.post_as(
            "/staff",
            actor,
            json!({
                "telegram_user_id": telegram_id,
                "display_name": display_name,
                "role": role,
            }),
        )
        .await
    }

    pub async fn add_student(&self, display_name: &str) -> User {
        self.add_student_to_group(display_name, &self.fixture.group)
            .await
    }

    pub async fn add_student_to_group(&self, display_name: &str, group: &StudentGroup) -> User {
        let student = named_student(self.next_telegram_id(), display_name, group);
        insert_user(&self.pool, &student).await;

        student
    }

    pub async fn add_outsider_teacher(&self, display_name: &str) -> User {
        let teacher = named_teacher(self.next_telegram_id(), display_name);
        insert_user(&self.pool, &teacher).await;

        teacher
    }

    /// Очередь должна учитывать второго преподавателя курса, хотя use case его добавления пока нет.
    /// Поэтому тестовая связка создаётся напрямую в БД.
    pub async fn add_course_teacher(&self, display_name: &str) -> User {
        let teacher = self.add_outsider_teacher(display_name).await;

        sqlx::query(
            "insert into teaching_course_teachers (teaching_course_id, teacher_id) values ($1, $2)",
        )
        .bind(self.fixture.course.id().as_uuid())
        .bind(teacher.id().as_uuid())
        .execute(&self.pool)
        .await
        .expect("attach teacher to the course");

        teacher
    }

    pub async fn remove_course_teacher(&self, teacher: &User) {
        sqlx::query(
            "delete from teaching_course_teachers where teaching_course_id = $1 and teacher_id = $2",
        )
        .bind(self.fixture.course.id().as_uuid())
        .bind(teacher.id().as_uuid())
        .execute(&self.pool)
        .await
        .expect("detach teacher from the course");
    }

    pub async fn rename_table_after_writing(&self, written: &str, renamed: &str) {
        sqlx::raw_sql(AssertSqlSafe(format!(
            r#"
            create function rename_after_writing() returns trigger language plpgsql as $$
            begin
                if to_regclass('{renamed}') is not null then
                    alter table {renamed} rename to {renamed}_unavailable;
                end if;
                return null;
            end
            $$;
            create trigger rename_after_writing after insert or update or delete on {written}
            for each statement execute function rename_after_writing();
            "#
        )))
        .execute(&self.pool)
        .await
        .expect("rename a table after the next write");
    }

    pub async fn add_admin(&self) -> User {
        let admin = named_admin(self.next_telegram_id(), "Admin");
        insert_user(&self.pool, &admin).await;

        admin
    }

    pub async fn add_group_to_course(&self, course: &TeachingCourse) -> StudentGroup {
        let group = student_group_in(
            &format!("TEST-GROUP-{}", self.next_telegram_id()),
            EducationLevel::Bachelor,
            1,
        );
        insert_group(&self.pool, &group).await;
        db::academic::add_teaching_course_group(&self.pool, course.id(), group.id())
            .await
            .expect("add the group to the course");

        group
    }

    pub async fn reload_course(&self, course: &TeachingCourse) -> TeachingCourse {
        db::academic::find_teaching_course_by_id(&self.pool, course.id())
            .await
            .expect("read the course")
            .expect("the course exists")
    }

    pub async fn add_outsider_student(&self) -> User {
        let group = student_group_in(
            &format!("TEST-GROUP-{}", self.next_telegram_id()),
            EducationLevel::Master,
            1,
        );
        insert_group(&self.pool, &group).await;

        self.add_student_to_group("Outsider", &group).await
    }

    pub async fn add_draft_assignment(&self) -> Assignment {
        let draft = wall_clock_assignment(&self.fixture.course, &self.fixture.teacher)
            .title("Rust lifetimes")
            .body("Not ready yet.")
            .max_attempts(1)
            .build();

        insert_assignment(&self.pool, &draft).await;

        draft
    }

    pub async fn add_scored_assignment(&self, max_score: u16) -> Assignment {
        let assignment = wall_clock_assignment(&self.fixture.course, &self.fixture.teacher)
            .title("Rust traits")
            .scored(max_score)
            .published()
            .build();

        insert_assignment(&self.pool, &assignment).await;

        assignment
    }

    pub async fn add_overdue_assignment(
        &self,
        days_since_deadline: i64,
        late_policy: Option<(u16, u16, u16)>,
    ) -> Assignment {
        let deadline_at = Utc::now() - Duration::days(days_since_deadline);
        let mut builder = wall_clock_assignment(&self.fixture.course, &self.fixture.teacher)
            .title("Overdue assignment")
            .created_at(Utc::now() - Duration::days(days_since_deadline + 1))
            .deadline_at(deadline_at);

        if let Some((period_days, penalty_per_period, max_periods)) = late_policy {
            builder = builder.scored(5).late_policy(
                period_days,
                penalty_per_period,
                max_periods,
                DeductedFrom::WorkScore,
            );
        }

        let assignment = builder.published().build();
        insert_assignment(&self.pool, &assignment).await;

        assignment
    }

    pub async fn add_late_assignment(
        &self,
        title: &str,
        max_score: u16,
        penalty_per_period: u16,
        deducted_from: DeductedFrom,
    ) -> Assignment {
        let assignment = wall_clock_assignment(&self.fixture.course, &self.fixture.teacher)
            .title(title)
            .created_at(Utc::now() - Duration::days(2))
            .deadline_at(Utc::now() - Duration::days(1))
            .scored(max_score)
            .late_policy(7, penalty_per_period, 4, deducted_from)
            .published()
            .build();
        insert_assignment(&self.pool, &assignment).await;

        assignment
    }

    pub async fn add_foreign_course(&self) -> CourseFixture {
        self.add_foreign_course_on(&self.fixture.subject).await
    }

    pub async fn add_foreign_course_on(&self, subject: &Subject) -> CourseFixture {
        let group = student_group_in(
            &format!("TEST-GROUP-{}", self.next_telegram_id()),
            EducationLevel::Bachelor,
            3,
        );
        insert_group(&self.pool, &group).await;

        let teacher = named_teacher(self.next_telegram_id(), "Foreign teacher");
        insert_user(&self.pool, &teacher).await;

        let course = teaching_course(subject, &[&group], &teacher);
        insert_course(&self.pool, &course).await;

        CourseFixture {
            group,
            teacher,
            course,
        }
    }

    pub async fn add_sandbox_subject(&self) -> Subject {
        let subject = subject("Sandbox");
        db::academic::insert_subject(&self.pool, &subject)
            .await
            .expect("insert subject");
        sqlx::query("update subjects set sandbox = true where id = $1")
            .bind(subject.id().as_uuid())
            .execute(&self.pool)
            .await
            .expect("mark the subject as a sandbox");

        subject
    }

    pub async fn add_course_on(&self, subject: &Subject) -> TeachingCourse {
        let course = teaching_course(subject, &[&self.fixture.group], &self.fixture.teacher);
        insert_course(&self.pool, &course).await;

        course
    }

    pub async fn add_sandbox_course(&self) -> TeachingCourse {
        let subject = self.add_sandbox_subject().await;

        self.add_course_on(&subject).await
    }

    pub async fn add_subject_course(&self, subject_name: &str) -> TeachingCourse {
        let subject = subject(subject_name);
        db::academic::insert_subject(&self.pool, &subject)
            .await
            .expect("insert subject");

        let course = teaching_course(&subject, &[&self.fixture.group], &self.fixture.teacher);
        insert_course(&self.pool, &course).await;

        course
    }

    pub async fn enable_makeup(&self, attempts: u16) -> CourseMakeup {
        self.enable_makeup_on(&self.fixture.course, attempts).await
    }

    pub async fn enable_makeup_on(&self, course: &TeachingCourse, attempts: u16) -> CourseMakeup {
        self.open_makeup(course, attempts, Utc::now()).await
    }

    pub async fn open_makeup(
        &self,
        course: &TeachingCourse,
        attempts: u16,
        now: DateTime<Utc>,
    ) -> CourseMakeup {
        self.application
            .courses
            .configure_makeup(ConfigureMakeup {
                course_id: course.id(),
                actor: self.teacher_actor(),
                expected: None,
                settings: Some(MakeupSettings {
                    until: now + Duration::days(30),
                    attempts,
                }),
                decisions: Vec::new(),
                now,
            })
            .await
            .expect("enable makeup")
            .expect("makeup is enabled")
    }

    pub async fn add_makeup_assignment(
        &self,
        title: &str,
        grading: Grading,
        makeup: MakeupTerms,
        deadline_at: DateTime<Utc>,
    ) -> Assignment {
        self.add_makeup_assignment_on(&self.fixture.course, title, grading, makeup, deadline_at)
            .await
    }

    pub async fn add_makeup_assignment_on(
        &self,
        course: &TeachingCourse,
        title: &str,
        grading: Grading,
        makeup: MakeupTerms,
        deadline_at: DateTime<Utc>,
    ) -> Assignment {
        let assignment = assignment_of(course, &self.fixture.teacher)
            .title(title)
            .created_at(deadline_at.min(Utc::now()).trunc_subsecs(6) - Duration::days(1))
            .deadline_at(deadline_at)
            .grading(grading)
            .makeup(makeup)
            .published()
            .build();
        insert_assignment(&self.pool, &assignment).await;

        assignment
    }

    pub async fn add_closed_assignment(
        &self,
        title: &str,
        max_score: u16,
        credit: u16,
    ) -> Assignment {
        let grading = Grading::scored(max_score).expect("valid scale");
        self.add_makeup_assignment(
            title,
            grading,
            MakeupTerms::scored(credit, grading).expect("valid credit"),
            Utc::now() - Duration::days(1),
        )
        .await
    }

    pub async fn add_published_assignment(
        &self,
        course: &TeachingCourse,
        author: &User,
        title: &str,
    ) -> Assignment {
        let assignment = wall_clock_assignment(course, author)
            .title(title)
            .published()
            .build();
        insert_assignment(&self.pool, &assignment).await;

        assignment
    }
}

pub fn test_file_policy() -> FilePolicy {
    FilePolicy {
        max_file_size_bytes: 5 * 1024 * 1024,
        max_files_per_assignment: 5,
        max_files_per_submission: 1,
    }
}

fn empty_request(builder: Builder) -> Request<Body> {
    builder.body(Body::empty()).expect("valid request")
}

fn json_request(builder: Builder, body: Value) -> Request<Body> {
    builder
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).expect("valid json")))
        .expect("valid request")
}

pub fn out_of(value: u16, max_score: u16) -> ReviewScore {
    ReviewScore { value, max_score }
}

#[track_caller]
pub fn submission_id(body: &Value) -> Uuid {
    id(body, "submission")
}

#[track_caller]
pub fn assignment_id(body: &Value) -> Uuid {
    id(body, "assignment")
}

#[track_caller]
fn id(body: &Value, resource: &str) -> Uuid {
    body["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{resource} id is missing in {body:#}"))
        .parse()
        .unwrap_or_else(|_| panic!("{resource} id is a uuid"))
}
