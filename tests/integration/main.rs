//! Все интеграционные тесты - один бинарник. Каждый файл в `tests/` компилируется в отдельный
//! процесс, а значит поднимает свой контейнер Postgres; здесь контейнер общий, а изоляция идёт
//! по базе на тест.

mod harness;

mod assignment_files;
mod assignment_teacher_queries;
mod assignments_api;
mod attempts_api;
mod bootstrap;
mod course_invites;
mod course_roster;
mod db_schema;
mod grading_api;
mod health_api;
mod makeup;
mod notification_outbox;
mod notification_worker;
mod pagination;
mod resilience;
mod review_api;
mod sandbox_review;
mod student_contacts;
mod student_read_api;
mod student_review_queue;
mod submission_repository;
mod submit_homework_api;
mod teacher_read_api;
mod telegram;
mod users_api;
