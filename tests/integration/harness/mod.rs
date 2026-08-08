use std::sync::atomic::{AtomicI64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::request::Builder;
use chrono::{DateTime, Duration, Utc};
use hw_bot::api::{self, ACTOR_HEADER, AppState};
use hw_bot::db;
use hw_bot::domain::{Assignment, EducationLevel, User};
use hw_bot::test_support::{
    OTHER_GROUP_CODE, named_admin, named_student, named_teacher, student_group_in,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt, ReuseDirective};
use tokio::sync::OnceCell;
use tower::ServiceExt;
use uuid::Uuid;

mod fixture;
mod response;

pub use fixture::Fixture;
pub use response::TestResponse;

use fixture::{insert_course, insert_group, insert_user, wall_clock_assignment};

const POSTGRES_PORT: u16 = 5432;

/// Именованный контейнер, который переиспользуется между прогонами. Имя обязательно: по нему
/// testcontainers находит уже запущенный контейнер вместо того, чтобы поднимать новый.
const CONTAINER_NAME: &str = "hw-bot-test-postgres";

/// Контейнер один на весь тестовый бинарник, база - на каждый
/// тест.
static POSTGRES: OnceCell<Postgres> = OnceCell::const_new();

struct Postgres {
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

impl Postgres {
    fn url(&self, database: &str) -> String {
        format!(
            "postgres://postgres:postgres@{}:{}/{database}",
            self.host, self.port
        )
    }
}

async fn postgres() -> &'static Postgres {
    POSTGRES
        .get_or_init(|| async {
            let container = GenericImage::new("postgres", "18.4-bookworm")
                .with_exposed_port(POSTGRES_PORT.tcp())
                .with_wait_for(WaitFor::message_on_stderr(
                    "database system is ready to accept connections",
                ))
                .with_env_var("POSTGRES_USER", "postgres")
                .with_env_var("POSTGRES_PASSWORD", "postgres")
                // Контейнер живёт в `static`, а статики не дропаются: без явного реюза каждый
                // прогон оставлял бы за собой ещё один Postgres. С реюзом он ровно один и
                // переживает прогон намеренно.
                .with_container_name(CONTAINER_NAME)
                .with_reuse(ReuseDirective::Always)
                // База одноразовая, долговечность не нужна. Без этого `create database` на
                // каждый тест гонит копию template1 через WAL с fsync.
                .with_cmd([
                    "postgres",
                    "-c",
                    "fsync=off",
                    "-c",
                    "full_page_writes=off",
                    "-c",
                    "synchronous_commit=off",
                    "-c",
                    "max_connections=200",
                ])
                .start()
                .await
                .expect("postgres container starts (is docker running?)");

            let host = container
                .get_host()
                .await
                .expect("container host")
                .to_string();
            let port = container
                .get_host_port_ipv4(POSTGRES_PORT.tcp())
                .await
                .expect("container port");

            let postgres = Postgres {
                _container: container,
                host,
                port,
            };
            drop_stale_databases(&postgres.url("postgres")).await;

            postgres
        })
        .await
}

/// Контейнер переиспользуется, поэтому базы прошлых прогонов убираем сами - иначе они копятся.
/// Ошибки глотаем: занятую параллельным прогоном базу дропнуть нельзя, и это нормально.
async fn drop_stale_databases(url: &str) {
    let Ok(mut conn) = PgConnection::connect(url).await else {
        return;
    };

    let stale: Vec<String> =
        sqlx::query_scalar(r"select datname from pg_database where datname like 'test\_%'")
            .fetch_all(&mut conn)
            .await
            .unwrap_or_default();

    for name in stale {
        let _ = sqlx::query(AssertSqlSafe(format!(
            r#"drop database if exists "{name}""#
        )))
        .execute(&mut conn)
        .await;
    }

    let _ = conn.close().await;
}

/// Образ печатает "ready to accept connections" ещё на этапе initdb, поэтому первый коннект
/// иногда упирается в закрытый порт. Дешевле переподключиться, чем ловить редкий флак.
async fn connect_with_retry(url: &str, max_connections: u32) -> PgPool {
    let mut last_error = None;

    for attempt in 0..10 {
        match PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await
        {
            Ok(pool) => return pool,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt + 1))).await;
            }
        }
    }

    panic!("cannot connect to {url}: {}", last_error.expect("an error"));
}

pub async fn fresh_database() -> PgPool {
    let postgres = postgres().await;
    let name = format!("test_{}", Uuid::now_v7().simple());

    // Долгоживущий admin-пул в `static` не годится: каждый `#[tokio::test]` крутится на своём
    // runtime, а сокеты пула привязаны к реактору того теста, который его создал, и умирают
    // вместе с ним. Поэтому коннект одноразовый и открывается на текущем runtime.
    // Имя базы нельзя передать bind-параметром, но оно только что сгенерировано из uuid.
    let mut admin = PgConnection::connect(&postgres.url("postgres"))
        .await
        .expect("connect to the maintenance database");

    sqlx::query(AssertSqlSafe(format!(r#"create database "{name}""#)))
        .execute(&mut admin)
        .await
        .expect("create a database for the test");

    admin
        .close()
        .await
        .expect("close the maintenance connection");

    // Тесты идут параллельно: пул на тест держим узким, иначе десяток параллельных тестов
    // выберет max_connections сервера.
    connect_with_retry(&postgres.url(&name), 4).await
}

pub async fn migrated_database() -> PgPool {
    let pool = fresh_database().await;
    db::run_migrations(&pool).await.expect("migrations apply");

    pool
}

pub struct TestApp {
    pub pool: PgPool,
    pub fixture: Fixture,
    router: Router,
    next_telegram_id: AtomicI64,
}

impl TestApp {
    pub async fn start() -> Self {
        let pool = migrated_database().await;
        let fixture = Fixture::insert(&pool).await;
        let router = api::router(AppState::new("hw-bot-test", pool.clone()));

        Self {
            pool,
            fixture,
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

    /// Telegram id студента из фикстуры.
    pub fn student(&self) -> i64 {
        self.fixture.student.telegram_user_id().as_i64()
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

    pub async fn post(&self, path: &str, body: Value) -> TestResponse {
        self.send(json_request(Request::post(path), body)).await
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
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");

        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("response body is json")
        };

        TestResponse { status, body }
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

    pub async fn register_student(
        &self,
        telegram_id: i64,
        display_name: &str,
        group_code: &str,
    ) -> TestResponse {
        self.post_as(
            "/students",
            telegram_id,
            json!({
                "display_name": display_name,
                "student_group_code": group_code,
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
        let student = named_student(self.next_telegram_id(), display_name, &self.fixture.group);
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
        let group = student_group_in(OTHER_GROUP_CODE, EducationLevel::Master, 1);
        insert_group(&self.pool, &group).await;

        let student = named_student(self.next_telegram_id(), "Outsider", &group);
        insert_user(&self.pool, &student).await;

        student
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

    /// Второй курс с собственным преподавателем, группой и заданием: нужен, чтобы проверить, что
    /// очередь ревью не смешивает курсы.
    pub async fn add_foreign_course_assignment(&self) -> (User, User, Assignment) {
        let group = student_group_in("B9199-02.03.21", EducationLevel::Bachelor, 3);
        insert_group(&self.pool, &group).await;

        let teacher = named_teacher(self.next_telegram_id(), "Foreign teacher");
        insert_user(&self.pool, &teacher).await;

        let student = named_student(self.next_telegram_id(), "Foreign student", &group);
        insert_user(&self.pool, &student).await;

        let course = hw_bot::test_support::teaching_course(&self.fixture.subject, &group, &teacher);
        insert_course(&self.pool, &course).await;

        let foreign_assignment = wall_clock_assignment(&course, &teacher)
            .title("Foreign assignment")
            .published()
            .build();
        db::assignments::insert(&self.pool, &foreign_assignment)
            .await
            .expect("insert foreign assignment");

        (teacher, student, foreign_assignment)
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
