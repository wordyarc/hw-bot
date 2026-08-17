use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration as StdDuration;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::request::Builder;
use chrono::{DateTime, Duration, Utc};
use hw_bot::api::{self, ACTOR_HEADER, AppState};
use hw_bot::app::{Actor, ApplicationContainer, FilePolicy};
use hw_bot::db;
use hw_bot::domain::{
    Assignment, EducationLevel, StudentGroup, StudentGroupId, TeachingCourse, User,
};
use hw_bot::health::HealthState;
use hw_bot::invites::{GroupInviteCodec, InviteSigningSecret};
use hw_bot::test_support::{
    named_admin, named_student, named_teacher, student_group_in, subject, teaching_course,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

mod database;
mod fixture;
mod response;

pub use database::{fresh_database, migrated_database};
pub use fixture::{CourseFixture, Fixture};
pub use response::TestResponse;

use fixture::{insert_course, insert_group, insert_user, wall_clock_assignment};

/// Фиксированный секрет: тесты должны выпускать ровно те же ссылки, что и приложение под
/// этим же `AppState`.
const INVITE_SIGNING_SECRET: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

pub fn invite_codec() -> GroupInviteCodec {
    GroupInviteCodec::new(InviteSigningSecret::from_base64url(INVITE_SIGNING_SECRET).unwrap())
}

pub struct TestApp {
    pub pool: PgPool,
    pub fixture: Fixture,
    pub application: ApplicationContainer,
    pub invites: GroupInviteCodec,
    router: Router,
    next_telegram_id: AtomicI64,
}

impl TestApp {
    pub async fn start() -> Self {
        Self::start_with_http_timeout(StdDuration::from_secs(10)).await
    }

    pub async fn start_with_http_timeout(request_timeout: StdDuration) -> Self {
        let pool = migrated_database().await;
        let fixture = Fixture::insert(&pool).await;
        let application = ApplicationContainer::new(pool.clone(), test_file_policy());
        let invites = invite_codec();
        let router = api::router(
            AppState::new(
                "hw-bot-test",
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
            // Выше диапазона, который тесты занимают литералами.
            next_telegram_id: AtomicI64::new(5000),
        }
    }

    pub fn assignment(&self) -> Uuid {
        self.fixture.assignment.id().as_uuid()
    }

    pub fn course(&self) -> Uuid {
        self.fixture.course.id().as_uuid()
    }

    /// Telegram id преподавателя курса из фикстуры.
    pub fn teacher(&self) -> i64 {
        self.fixture.teacher.telegram_user_id().as_i64()
    }

    pub fn teacher_actor(&self) -> Actor {
        Actor::new(self.fixture.teacher.id())
    }

    /// Telegram id студента из фикстуры.
    pub fn student(&self) -> i64 {
        self.fixture.student.telegram_user_id().as_i64()
    }

    pub fn student_actor(&self) -> Actor {
        Actor::new(self.fixture.student.id())
    }

    pub fn next_telegram_id(&self) -> i64 {
        self.next_telegram_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Без актора: только health и справочники.
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

    pub async fn post(&self, path: &str, body: Value) -> TestResponse {
        self.send(json_request(Request::post(path), body)).await
    }

    pub async fn post_raw(&self, path: &str, body: Vec<u8>) -> TestResponse {
        let request = Request::post(path)
            .header("content-type", "application/json")
            .body(Body::from(body))
            .expect("valid request");
        self.send(request).await
    }

    pub async fn post_as(&self, path: &str, actor: i64, body: Value) -> TestResponse {
        self.send(json_request(
            Request::post(path).header(ACTOR_HEADER, actor),
            body,
        ))
        .await
    }

    /// Команда, у которой весь вход - это актор и путь.
    pub async fn post_empty_as(&self, path: &str, actor: i64) -> TestResponse {
        self.send(empty_request(
            Request::post(path).header(ACTOR_HEADER, actor),
        ))
        .await
    }

    /// Для тестов, которые проверяют разбор самого заголовка актора.
    pub async fn post_with_actor_header(
        &self,
        path: &str,
        actor: &str,
        body: Value,
    ) -> TestResponse {
        self.send(json_request(
            Request::post(path).header(ACTOR_HEADER, actor),
            body,
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

    /// Для тестов, которые проверяют разбор самого тела запроса.
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

    /// Сдача, уже лежащая в очереди: дальше почти всегда нужен её id.
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

    /// Для тестов, которые проверяют разбор и валидацию самого тела запроса.
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
        self.post_as(
            &format!("/submissions/{submission}/review"),
            telegram_id,
            json!({ "decision": decision, "comment": comment }),
        )
        .await
    }

    /// Действующая ссылка группы: generation читается из БД, поэтому после ротации токен
    /// нужно взять заново.
    pub async fn invite_token(&self, group_id: StudentGroupId) -> String {
        let generation = db::group_invites::find_generation(&self.pool, group_id)
            .await
            .expect("read invite generation")
            .expect("group has an invite");

        self.invites.encode(group_id, generation)
    }

    pub async fn group_invite_token(&self) -> String {
        self.invite_token(self.fixture.group.id()).await
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

    /// Преподаватель без привязки к курсу фикстуры: нужен, чтобы проверять, что чужие сдачи ему
    /// не видны и не подчиняются.
    pub async fn add_outsider_teacher(&self, display_name: &str) -> User {
        let teacher = named_teacher(self.next_telegram_id(), display_name);
        insert_user(&self.pool, &teacher).await;

        teacher
    }

    /// Второй преподаватель того же курса. Пишем связку напрямую: отдельного use case «добавить
    /// ревьюера» ещё нет, а очередь ревью обязана его видеть уже сейчас.
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

    pub async fn add_admin(&self) -> User {
        let admin = named_admin(self.next_telegram_id(), "Admin");
        insert_user(&self.pool, &admin).await;

        admin
    }

    /// Студент чужой группы вместе с самой группой.
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

        db::assignments::insert(&self.pool, &draft)
            .await
            .expect("insert draft assignment");

        draft
    }

    pub async fn add_foreign_course(&self) -> CourseFixture {
        let group = student_group_in(
            &format!("TEST-GROUP-{}", self.next_telegram_id()),
            EducationLevel::Bachelor,
            3,
        );
        insert_group(&self.pool, &group).await;

        let teacher = named_teacher(self.next_telegram_id(), "Foreign teacher");
        insert_user(&self.pool, &teacher).await;

        let course = teaching_course(&self.fixture.subject, &group, &teacher);
        insert_course(&self.pool, &course).await;

        CourseFixture {
            group,
            teacher,
            course,
        }
    }

    /// Второй предмет той же группы: студент учится на нескольких курсах сразу.
    pub async fn add_subject_course(&self, subject_name: &str) -> TeachingCourse {
        let subject = subject(subject_name);
        db::academic::insert_subject(&self.pool, &subject)
            .await
            .expect("insert subject");

        let course = teaching_course(&subject, &self.fixture.group, &self.fixture.teacher);
        insert_course(&self.pool, &course).await;

        course
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
        db::assignments::insert(&self.pool, &assignment)
            .await
            .expect("insert published assignment");

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
