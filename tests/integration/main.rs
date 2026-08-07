//! Все интеграционные тесты - один бинарник. Каждый файл в `tests/` компилируется в отдельный
//! процесс, а значит поднимает свой контейнер Postgres; здесь контейнер общий, а изоляция идёт
//! по базе на тест.

mod harness;

mod db_schema;
mod health_api;
mod review_api;
mod submission_repository;
mod submit_homework_api;
mod users_api;
