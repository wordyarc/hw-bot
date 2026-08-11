//! Все интеграционные тесты - один бинарник. Каждый файл в `tests/` компилируется в отдельный
//! процесс, а значит поднимает свой контейнер Postgres; здесь контейнер общий, а изоляция идёт
//! по базе на тест.

mod harness;

mod actor_api;
mod assignment_teacher_queries;
mod assignments_api;
mod attempts_api;
mod db_schema;
mod health_api;
mod notification_outbox;
mod notification_worker;
mod review_api;
mod student_read_api;
mod submission_repository;
mod submit_homework_api;
mod teacher_read_api;
mod telegram_bot;
mod users_api;
