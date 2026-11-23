use std::collections::BTreeSet;

use chrono::Utc;
use hw_bot::app::{Actor, CreateCourse, CreateStudentGroup, ErrorCode};
use hw_bot::bot::{app_error_message, callbacks, texts};
use hw_bot::db;
use hw_bot::domain::{
    AssignmentStatus, EducationLevel, Role, StudentGroup, TeachingCourse, TelegramUserId, User,
};
use serde_json::{Value, json};

use crate::harness::TestApp;
use crate::harness::telegram::*;
use crate::harness::telegram_scenario::{Button, TelegramScenario};

const REQUEST_ID: i32 = 1;

struct Section {
    app: TestApp,
    telegram: TelegramScenario,
    admin: User,
    user: i64,
    cancellation: tokio_util::sync::CancellationToken,
    task: tokio::task::JoinHandle<Result<(), hw_bot::bot::TelegramBotError>>,
}

impl Section {
    async fn open() -> Self {
        let app = TestApp::start().await;
        let telegram = TelegramScenario::new().await;
        let admin = app.add_admin().await;
        let (cancellation, task) = start_bot(&app, &telegram.server);
        let user = admin.telegram_user_id().as_i64();
        let section = Self {
            app,
            telegram,
            admin,
            user,
            cancellation,
            task,
        };
        section.say("/admin").await;
        section
    }

    fn actor(&self) -> Actor {
        Actor::new(self.admin.id())
    }

    async fn say(&self, text: &str) {
        self.telegram.message(self.user, text);
        self.telegram.flush(self.user).await;
    }

    async fn press(&self, label: &str) -> Button {
        let button = self.telegram.button_with_text(self.user, label).await;
        self.telegram.click(self.user, &button);
        self.telegram.flush(self.user).await;
        button
    }

    fn texts(&self) -> Vec<String> {
        self.telegram.texts(self.user)
    }

    fn said(&self, text: &str) -> bool {
        self.texts().iter().any(|sent| sent == text)
    }

    fn mentioned(&self, fact: &str) -> bool {
        self.texts().iter().any(|sent| sent.contains(fact))
    }

    fn last_bottom_menu(&self) -> Vec<String> {
        self.telegram
            .messages(self.user)
            .iter()
            .rev()
            .find_map(|message| message["reply_markup"]["keyboard"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .flat_map(|row| row.as_array().unwrap().clone())
            .map(|button| button["text"].as_str().unwrap().to_owned())
            .collect()
    }

    fn labels(&self) -> Vec<String> {
        self.telegram
            .messages(self.user)
            .iter()
            .rev()
            .find(|message| message["reply_markup"]["inline_keyboard"].is_array())
            .map(inline_labels)
            .unwrap_or_default()
    }

    async fn group(&self, code: &str) -> StudentGroup {
        self.app
            .application
            .admin
            .create_group(CreateStudentGroup {
                actor: self.actor(),
                code: code.to_owned(),
                education_level: EducationLevel::Bachelor,
                study_year: 2,
            })
            .await
            .expect("create group")
    }

    async fn course(&self, name: &str, groups: &[&StudentGroup]) -> TeachingCourse {
        let admin = &self.app.application.admin;
        let subject = admin
            .create_subject(self.actor(), name.to_owned())
            .await
            .expect("create subject");
        admin
            .create_course(CreateCourse {
                actor: self.actor(),
                subject_id: subject.id(),
                group_ids: groups.iter().map(|group| group.id()).collect(),
                teacher_ids: BTreeSet::from([self.app.fixture.teacher.id()]),
                created_at: Utc::now(),
            })
            .await
            .expect("create course")
    }

    async fn open_student(&self, name: &str) {
        self.press(texts::BUTTON_ADMIN_STUDENTS).await;
        self.say(name).await;
        let label = self
            .labels()
            .into_iter()
            .find(|label| label.contains(name))
            .expect("the student is in the results");
        self.press(&label).await;
    }

    async fn open_course(&self, label: &str) {
        self.say("/admin").await;
        self.press(texts::BUTTON_ADMIN_COURSES).await;
        let label = self
            .labels()
            .into_iter()
            .find(|shown| shown.contains(label))
            .expect("the course is listed");
        self.press(&label).await;
    }

    async fn stop(mut self) {
        stop_bot(self.cancellation, &mut self.task).await;
    }
}

fn inline_labels(message: &Value) -> Vec<String> {
    message["reply_markup"]["inline_keyboard"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|row| row.as_array().unwrap().clone())
        .map(|button| button["text"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_admin_opens_the_section_from_the_bottom_menu() {
    let section = Section::open().await;

    section.say("/menu").await;
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN.to_owned())
    );
    section.say(texts::BUTTON_ADMIN).await;

    assert!(section.said(texts::ADMIN_HOME));
    for label in [
        texts::BUTTON_ADMIN_STUDENTS,
        texts::BUTTON_ADMIN_TEACHERS,
        texts::BUTTON_ADMIN_GROUPS,
        texts::BUTTON_ADMIN_SUBJECTS,
        texts::BUTTON_ADMIN_COURSES,
    ] {
        assert!(section.labels().contains(&label.to_owned()), "{label}");
    }
    section.stop().await;
}

#[tokio::test]
async fn a_teacher_has_no_menu_item_and_is_refused_everywhere() {
    let app = TestApp::start().await;
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();
    let denied = app_error_message(ErrorCode::PermissionDenied);

    telegram.message(teacher, "/menu");
    telegram.flush(teacher).await;
    let menu = telegram
        .messages(teacher)
        .iter()
        .rev()
        .find_map(|message| message["reply_markup"]["keyboard"].as_array().cloned())
        .expect("the bottom menu was sent");
    assert!(!json!(menu).to_string().contains(texts::BUTTON_ADMIN));

    for input in ["/admin", texts::BUTTON_ADMIN] {
        telegram.message(teacher, input);
        telegram.flush(teacher).await;
    }
    let refusals = |telegram: &TelegramScenario| {
        telegram
            .texts(teacher)
            .iter()
            .filter(|text| *text == denied)
            .count()
    };
    assert_eq!(refusals(&telegram), 2);
    assert!(
        !telegram
            .texts(teacher)
            .iter()
            .any(|text| text == texts::ADMIN_HOME)
    );

    telegram.message(teacher, "/courses");
    telegram.flush(teacher).await;
    let stale = telegram
        .button_matching(teacher, |_| true)
        .await
        .with_data(format!("{}h", callbacks::ADMIN_PREFIX));
    telegram.click(teacher, &stale);
    telegram.flush(teacher).await;
    assert_eq!(refusals(&telegram), 3);
    stop_bot(cancellation, &mut task).await;
}

#[tokio::test]
async fn a_search_pages_through_students_and_opens_a_card() {
    let section = Section::open().await;
    for index in 0..12 {
        section
            .app
            .add_student(&format!("Искомый {index:02}"))
            .await;
    }

    section.press(texts::BUTTON_ADMIN_STUDENTS).await;
    section.say("искомый").await;
    assert!(
        section
            .labels()
            .iter()
            .any(|label| label.contains("Искомый 00"))
    );

    section.press(texts::BUTTON_NEXT_PAGE).await;
    let second_page = section.labels();
    assert!(second_page.iter().any(|label| label.contains("Искомый 11")));
    assert!(!second_page.iter().any(|label| label.contains("Искомый 00")));

    let label = second_page
        .iter()
        .find(|label| label.contains("Искомый 11"))
        .unwrap();
    section.press(label).await;
    assert!(section.mentioned(section.app.fixture.group.code().as_str()));
    assert!(
        section
            .labels()
            .contains(&texts::BUTTON_ADMIN_CHANGE_GROUP.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn a_too_short_query_keeps_the_search_step() {
    let section = Section::open().await;
    let student = section.app.add_student("Единственный").await;

    section.press(texts::BUTTON_ADMIN_STUDENTS).await;
    section.say("е").await;
    assert!(section.said(app_error_message(ErrorCode::InvalidInput)));
    section.say("единств").await;

    assert!(
        section
            .labels()
            .iter()
            .any(|label| label.contains("Единственный"))
    );
    assert_eq!(student.display_name().as_str(), "Единственный");
    section.stop().await;
}

#[tokio::test]
async fn fixing_a_name_reminds_to_tell_the_student_and_returns_the_menu() {
    let section = Section::open().await;
    let student = section.app.add_student("Опечатка Имя").await;

    section.open_student("Опечатка").await;
    section.press(texts::BUTTON_ADMIN_FIX_NAME).await;
    section.say("  Верное \n Имя ").await;

    let stored = db::users::find_by_id(&section.app.pool, student.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.display_name().as_str(), "Верное Имя");
    assert!(section.said(&texts::admin_student_renamed()));
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN.to_owned())
    );
    assert!(
        section
            .telegram
            .messages(student.telegram_user_id().as_i64())
            .is_empty()
    );
    section.stop().await;
}

#[tokio::test]
async fn a_card_with_a_submission_names_the_reason_and_offers_no_group_change() {
    let section = Section::open().await;
    section
        .app
        .queued_submission(section.app.student(), "pull/1")
        .await;

    section.open_student("Student").await;

    assert!(section.mentioned("Группу сменить нельзя"));
    assert!(
        !section
            .labels()
            .contains(&texts::BUTTON_ADMIN_CHANGE_GROUP.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn a_group_change_stores_the_marked_subjects() {
    let section = Section::open().await;
    let group = section.group("NEW-GROUP").await;
    let first = section.course("Алгоритмы", &[&group]).await;
    section.course("Базы данных", &[&group]).await;
    let student = &section.app.fixture.student;

    section.open_student("Student").await;
    section.press(texts::BUTTON_ADMIN_CHANGE_GROUP).await;
    section.press("NEW-GROUP").await;
    section
        .press(&texts::subject_choice_button("Алгоритмы", false))
        .await;
    section.press(texts::BUTTON_GROUPS_DONE).await;

    let stored = db::users::find_by_id(&section.app.pool, student.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.student_group_id(), Some(group.id()));
    let enrolled: Vec<uuid::Uuid> = sqlx::query_scalar(
        "select teaching_course_id from teaching_course_students where student_id = $1",
    )
    .bind(student.id().as_uuid())
    .fetch_all(&section.app.pool)
    .await
    .unwrap();
    assert_eq!(enrolled, [first.id().as_uuid()]);
    assert!(section.said(&texts::admin_student_moved()));
    assert!(section.telegram.messages(section.app.student()).is_empty());
    section.stop().await;
}

#[tokio::test]
async fn a_stale_subject_button_changes_nothing() {
    let section = Section::open().await;
    let group = section.group("NEW-GROUP").await;
    section.course("Алгоритмы", &[&group]).await;

    section.open_student("Student").await;
    section.press(texts::BUTTON_ADMIN_CHANGE_GROUP).await;
    section.press("NEW-GROUP").await;
    let done = section
        .telegram
        .button_with_text(section.user, texts::BUTTON_GROUPS_DONE)
        .await;
    section.say("/menu").await;
    section.telegram.click(section.user, &done);
    section.telegram.flush(section.user).await;

    assert!(section.said(texts::STALE_BUTTON));
    let stored = db::users::find_by_id(&section.app.pool, section.app.fixture.student.id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.student_group_id(),
        Some(section.app.fixture.group.id())
    );
    section.stop().await;
}

#[tokio::test]
async fn removing_from_a_subject_asks_first_and_reminds_to_tell_the_student() {
    let section = Section::open().await;
    let subject = section.app.fixture.subject.name().as_str().to_owned();

    section.open_student("Student").await;
    section
        .press(&texts::admin_remove_from_subject_button(&subject))
        .await;
    assert!(section.mentioned(&subject));
    let before: i64 = count_enrollments(&section).await;
    assert_eq!(before, 1);
    section.press(texts::BUTTON_ADMIN_CONFIRM_REMOVE).await;

    assert_eq!(count_enrollments(&section).await, 0);
    assert!(section.said(&texts::admin_student_unenrolled()));
    section.stop().await;
}

async fn count_enrollments(section: &Section) -> i64 {
    sqlx::query_scalar("select count(*) from teaching_course_students where student_id = $1")
        .bind(section.app.fixture.student.id().as_uuid())
        .fetch_one(&section.app.pool)
        .await
        .unwrap()
}

fn shared(request_id: i32, user_id: i64) -> Value {
    json!({
        "users_shared": {
            "request_id": request_id,
            "users": [{"user_id": user_id, "first_name": "Анна", "last_name": "Ким"}]
        }
    })
}

async fn stored_by_telegram(section: &Section, telegram_id: i64) -> Option<User> {
    db::users::find_by_telegram_user_id(
        &section.app.pool,
        TelegramUserId::new(telegram_id).unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_teacher_picked_in_telegram_is_registered_with_the_confirmed_name() {
    let section = Section::open().await;

    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press(texts::BUTTON_ADMIN_NEW_TEACHER).await;
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN_PICK_USER.to_owned())
    );
    section
        .telegram
        .message_with(section.user, shared(REQUEST_ID, 77_001));
    section.telegram.flush(section.user).await;
    section
        .press(&texts::admin_keep_name_button("Анна Ким"))
        .await;

    let teacher = stored_by_telegram(&section, 77_001)
        .await
        .expect("registered");
    assert_eq!(teacher.display_name().as_str(), "Анна Ким");
    assert!(matches!(teacher.role(), Role::Teacher));
    assert!(section.said(texts::ADMIN_TEACHER_REGISTERED));
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn an_answer_with_a_foreign_request_id_is_ignored() {
    let section = Section::open().await;

    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press(texts::BUTTON_ADMIN_NEW_TEACHER).await;
    section
        .telegram
        .message_with(section.user, shared(REQUEST_ID + 1, 77_002));
    section.telegram.flush(section.user).await;

    assert!(!section.mentioned(texts::ADMIN_TEACHER_NAME_PROMPT));
    assert!(stored_by_telegram(&section, 77_002).await.is_none());
    section.stop().await;
}

#[tokio::test]
async fn a_shared_user_outside_the_step_is_ignored() {
    let section = Section::open().await;

    section
        .telegram
        .message_with(section.user, shared(REQUEST_ID, 77_003));
    section.telegram.flush(section.user).await;

    assert!(!section.mentioned(texts::ADMIN_TEACHER_NAME_PROMPT));
    assert!(stored_by_telegram(&section, 77_003).await.is_none());
    section.stop().await;
}

#[tokio::test]
async fn a_teacher_can_be_registered_by_a_typed_id() {
    let section = Section::open().await;

    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press(texts::BUTTON_ADMIN_NEW_TEACHER).await;
    section.say("не число").await;
    assert!(section.said(texts::ADMIN_TEACHER_ID_INVALID));
    section.say("77004").await;
    section.say("Пётр Ли").await;

    let teacher = stored_by_telegram(&section, 77_004)
        .await
        .expect("registered");
    assert_eq!(teacher.display_name().as_str(), "Пётр Ли");
    section.stop().await;
}

#[tokio::test]
async fn a_taken_id_says_the_user_is_registered_and_changes_nothing() {
    let section = Section::open().await;
    let student = section.app.student();

    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press(texts::BUTTON_ADMIN_NEW_TEACHER).await;
    section.say(&student.to_string()).await;
    section.say("Чужое имя").await;

    assert!(section.said(texts::ADMIN_USER_ALREADY_REGISTERED));
    assert!(!section.said(app_error_message(ErrorCode::AlreadyRegistered)));
    let stored = stored_by_telegram(&section, student).await.unwrap();
    assert_eq!(stored.display_name().as_str(), "Student");
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn subjects_of_a_teacher_are_toggled_and_the_last_teacher_stays() {
    let section = Section::open().await;
    let second = section.app.add_outsider_teacher("Второй").await;
    let course_label = |checked| {
        texts::admin_check_button(
            &texts::course_button(
                section.app.fixture.subject.name().as_str(),
                section.app.fixture.group.code().as_str(),
            ),
            checked,
        )
    };

    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press("Второй").await;
    section.press(texts::BUTTON_ADMIN_TEACHER_COURSES).await;
    section.press(&course_label(false)).await;
    let reloaded = section.app.reload_course(&section.app.fixture.course).await;
    assert!(reloaded.teacher_ids().contains(&second.id()));

    section.press(&course_label(true)).await;
    let reloaded = section.app.reload_course(&section.app.fixture.course).await;
    assert!(!reloaded.teacher_ids().contains(&second.id()));

    section.say("/admin").await;
    section.press(texts::BUTTON_ADMIN_TEACHERS).await;
    section.press("Teacher").await;
    section.press(texts::BUTTON_ADMIN_TEACHER_COURSES).await;
    section.press(&course_label(true)).await;

    assert!(section.said(app_error_message(ErrorCode::LastCourseTeacher)));
    let reloaded = section.app.reload_course(&section.app.fixture.course).await;
    assert!(
        reloaded
            .teacher_ids()
            .contains(&section.app.fixture.teacher.id())
    );
    section.stop().await;
}

#[tokio::test]
async fn a_group_is_created_renamed_and_moved_to_another_year() {
    let section = Section::open().await;

    section.press(texts::BUTTON_ADMIN_GROUPS).await;
    section.press(texts::BUTTON_ADMIN_NEW_GROUP).await;
    section.say("Б-101").await;
    section
        .press(texts::education_level_name(EducationLevel::Bachelor))
        .await;
    section.press(&texts::admin_year_button(2)).await;
    assert!(section.said(texts::ADMIN_GROUP_CREATED));
    assert!(
        section
            .last_bottom_menu()
            .contains(&texts::BUTTON_ADMIN.to_owned())
    );

    section.press(texts::BUTTON_ADMIN_RENAME).await;
    section.say("Б-102").await;
    assert!(section.said(texts::ADMIN_GROUP_RENAMED));

    section.press(texts::BUTTON_ADMIN_STUDY_YEAR).await;
    section.press(&texts::admin_year_button(3)).await;
    assert!(section.said(&texts::admin_group_card(
        "Б-102",
        EducationLevel::Bachelor,
        3
    )));
    section.stop().await;
}

#[tokio::test]
async fn a_taken_group_code_is_explained_and_keeps_the_step() {
    let section = Section::open().await;
    let taken = section.app.fixture.group.code().as_str().to_owned();
    let group = section.group("Б-101").await;

    section.press(texts::BUTTON_ADMIN_GROUPS).await;
    section.press("Б-101").await;
    section.press(texts::BUTTON_ADMIN_RENAME).await;
    section.say(&taken).await;
    assert!(section.said(app_error_message(ErrorCode::NameTaken)));
    section.say("Б-103").await;

    let stored = section
        .app
        .application
        .admin
        .find_group(section.actor(), group.id())
        .await
        .unwrap();
    assert_eq!(stored.code().as_str(), "Б-103");
    section.stop().await;
}

#[tokio::test]
async fn a_subject_is_created_and_renamed() {
    let section = Section::open().await;

    section.press(texts::BUTTON_ADMIN_SUBJECTS).await;
    section.press(texts::BUTTON_ADMIN_NEW_SUBJECT).await;
    section.say("Алгоритмы").await;
    assert!(section.said(texts::ADMIN_SUBJECT_CREATED));
    section.press(texts::BUTTON_ADMIN_RENAME).await;
    section.say("Алгоритмы и структуры").await;

    assert!(section.said(&texts::admin_subject_card("Алгоритмы и структуры", false)));
    section.stop().await;
}

#[tokio::test]
async fn the_course_wizard_creates_a_course_with_an_invite() {
    let section = Section::open().await;
    let group = section.group("G-WIZARD").await;
    let subject = section
        .app
        .application
        .admin
        .create_subject(section.actor(), "Мастер".to_owned())
        .await
        .unwrap();

    section.press(texts::BUTTON_ADMIN_COURSES).await;
    section.press(texts::BUTTON_ADMIN_NEW_COURSE).await;
    section.press("Мастер").await;
    section.press(texts::BUTTON_GROUPS_DONE).await;
    assert!(section.said(texts::ADMIN_CHOOSE_AT_LEAST_ONE));
    section
        .press(&texts::admin_check_button("G-WIZARD", false))
        .await;
    section.press(texts::BUTTON_GROUPS_DONE).await;
    section
        .press(&texts::admin_check_button("Teacher", false))
        .await;
    section.press(texts::BUTTON_GROUPS_DONE).await;

    assert!(section.said(texts::ADMIN_COURSE_CREATED));
    let free = section
        .app
        .application
        .admin
        .groups_free_for_subject(section.actor(), subject.id(), Default::default())
        .await
        .unwrap();
    assert!(!free.items.iter().any(|item| item.id() == group.id()));
    let generation: i64 = sqlx::query_scalar(
        "select generation from teaching_course_invites
         join teaching_courses on teaching_courses.id = teaching_course_id
         where teaching_courses.subject_id = $1",
    )
    .bind(subject.id().as_uuid())
    .fetch_one(&section.app.pool)
    .await
    .unwrap();
    assert_eq!(generation, 1);
    section.stop().await;
}

#[tokio::test]
async fn cancel_leaves_the_course_wizard_without_a_course() {
    let section = Section::open().await;
    section.group("G-WIZARD").await;
    section
        .app
        .application
        .admin
        .create_subject(section.actor(), "Мастер".to_owned())
        .await
        .unwrap();

    section.press(texts::BUTTON_ADMIN_COURSES).await;
    section.press(texts::BUTTON_ADMIN_NEW_COURSE).await;
    section.press("Мастер").await;
    let stale = section
        .telegram
        .button_with_text(section.user, &texts::admin_check_button("G-WIZARD", false))
        .await;
    section.say("/cancel").await;
    section.telegram.click(section.user, &stale);
    section.telegram.flush(section.user).await;

    assert!(section.said(texts::ACTION_CANCELLED));
    assert!(section.said(texts::STALE_BUTTON));
    let courses: i64 = sqlx::query_scalar("select count(*) from teaching_courses")
        .fetch_one(&section.app.pool)
        .await
        .unwrap();
    assert_eq!(courses, 1);
    section.stop().await;
}

#[tokio::test]
async fn a_group_is_added_and_the_admin_assigns_themselves() {
    let section = Section::open().await;
    let group = section.group("G-ADD").await;
    let course_name = section.app.fixture.subject.name().as_str().to_owned();

    section.open_course(&course_name).await;
    section.press(texts::BUTTON_ADMIN_ADD_GROUP).await;
    section.press("G-ADD").await;
    let reloaded = section.app.reload_course(&section.app.fixture.course).await;
    assert!(reloaded.includes_group(group.id()));

    section.press(texts::BUTTON_ADMIN_ASSIGN_SELF).await;
    let reloaded = section.app.reload_course(&section.app.fixture.course).await;
    assert!(reloaded.teacher_ids().contains(&section.admin.id()));
    assert!(
        !section
            .labels()
            .contains(&texts::BUTTON_ADMIN_ASSIGN_SELF.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn closing_rereads_the_numbers_and_needs_a_second_confirmation() {
    let section = Section::open().await;
    let course_name = section.app.fixture.subject.name().as_str().to_owned();

    section.open_course(&course_name).await;
    section.press(texts::BUTTON_ADMIN_CLOSE_COURSE).await;
    assert!(section.said(&texts::admin_close_confirmation(&course_name, 1, 0)));

    section
        .app
        .queued_submission(section.app.student(), "pull/1")
        .await;
    section.press(texts::BUTTON_ADMIN_CONFIRM_CLOSE).await;

    assert!(section.mentioned(texts::ADMIN_CLOSE_NUMBERS_CHANGED));
    assert!(section.mentioned(&texts::admin_close_confirmation(&course_name, 1, 1)));
    let open = section.app.reload_course(&section.app.fixture.course).await;
    assert!(!open.is_closed());

    section.press(texts::BUTTON_ADMIN_CONFIRM_CLOSE).await;

    assert!(section.said(&texts::admin_course_closed(1, 1)));
    let closed = section.app.reload_course(&section.app.fixture.course).await;
    assert!(closed.is_closed());
    let assignment =
        db::assignments::find_by_id(&section.app.pool, section.app.fixture.assignment.id())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(assignment.status(), AssignmentStatus::Archived);
    section.stop().await;
}

#[tokio::test]
async fn a_closed_course_is_returned_from_its_card() {
    let section = Section::open().await;
    let course = section.app.fixture.course.id();
    section
        .app
        .application
        .admin
        .close_course(section.actor(), course, Utc::now())
        .await
        .unwrap();

    section.press(texts::BUTTON_ADMIN_COURSES).await;
    section.press(texts::BUTTON_ADMIN_CLOSED_COURSES).await;
    let label = section
        .labels()
        .into_iter()
        .find(|label| label.contains(section.app.fixture.subject.name().as_str()))
        .unwrap();
    section.press(&label).await;
    section.press(texts::BUTTON_ADMIN_REOPEN_COURSE).await;

    assert!(section.said(texts::ADMIN_COURSE_REOPENED));
    let reopened = section.app.reload_course(&section.app.fixture.course).await;
    assert!(!reopened.is_closed());
    section.stop().await;
}

#[tokio::test]
async fn a_sandbox_course_offers_no_closing() {
    let section = Section::open().await;
    section.app.add_sandbox_course().await;

    section.open_course("Sandbox").await;

    assert!(
        !section
            .labels()
            .contains(&texts::BUTTON_ADMIN_CLOSE_COURSE.to_owned())
    );
    section.stop().await;
}

#[tokio::test]
async fn a_closed_course_screen_has_no_closed_actions_and_lives_in_its_own_list() {
    let app = TestApp::start().await;
    let draft = app.add_draft_assignment().await;
    let admin = app.add_admin().await;
    app.application
        .admin
        .close_course(Actor::new(admin.id()), app.fixture.course.id(), Utc::now())
        .await
        .unwrap();
    let telegram = TelegramScenario::new().await;
    let (cancellation, mut task) = start_bot(&app, &telegram.server);
    let teacher = app.teacher();

    telegram.message(teacher, "/courses");
    telegram.flush(teacher).await;
    let closed_list = telegram
        .button_with_text(teacher, texts::BUTTON_CLOSED_COURSES)
        .await;
    telegram.click(teacher, &closed_list);
    telegram.flush(teacher).await;
    telegram
        .click_button(teacher, &cb(callbacks::COURSE_PREFIX, app.course()))
        .await;

    let course_screen = telegram
        .button_matching(teacher, |data| data.starts_with(callbacks::GRADES_PREFIX))
        .await;
    let labels = {
        let message = telegram.message_by_id(teacher, course_screen.message_id);
        inline_labels(&message)
    };
    for hidden in [
        texts::BUTTON_CREATE_ASSIGNMENT,
        texts::BUTTON_COURSE_INVITE,
        texts::BUTTON_MAKEUP_OFF,
    ] {
        assert!(!labels.contains(&hidden.to_owned()), "{hidden}");
    }
    assert!(labels.contains(&texts::BUTTON_GRADEBOOK.to_owned()));
    assert!(
        telegram
            .texts(teacher)
            .iter()
            .any(|text| text.contains(texts::COURSE_IS_CLOSED))
    );

    telegram
        .click_button(teacher, &cb(callbacks::MANAGE_PREFIX, draft.id().as_uuid()))
        .await;
    let actions = telegram
        .button_matching(teacher, |data| data.starts_with(callbacks::ARCHIVE_PREFIX))
        .await;
    let labels = inline_labels(&telegram.message_by_id(teacher, actions.message_id));
    assert!(!labels.contains(&texts::BUTTON_PUBLISH.to_owned()));
    stop_bot(cancellation, &mut task).await;
}
