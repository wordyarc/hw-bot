use teloxide::payloads::SendMessageSetters;
use teloxide::types::{MessageId, UsersShared};

use super::reset_dialogue;
use crate::app::AppResult;
use crate::bot::callbacks::{AdminNav, AdminScreen, AdminStep};
use crate::bot::context::{ActorChat, TextStep};
use crate::bot::state::{AdminFlow, BotState, FlowState};
use crate::bot::texts;
use crate::bot::views::{admin_home_keyboard, admin_keyboard, send_screen};
use crate::bot::{HandlerResult, app_error_message};

mod lifecycle;
mod structure;
mod students;
mod teachers;

pub(super) async fn open(screen: ActorChat<'_>, nav: AdminNav) -> HandlerResult {
    match nav {
        AdminNav::Home => show_home(screen).await,
        AdminNav::SearchStudents => students::ask_query(screen).await,
        AdminNav::Student(id) => students::show_card(screen, id).await,
        AdminNav::RenameStudent(id) => students::ask_name(screen, id).await,
        AdminNav::ChangeGroup(id, page) => students::choose_new_group(screen, id, page).await,
        AdminNav::Unenroll(student, course) => {
            students::ask_unenroll(screen, student, course).await
        }
        AdminNav::Teachers(page) => teachers::show_list(screen, page).await,
        AdminNav::Teacher(id) => teachers::show_card(screen, id).await,
        AdminNav::NewTeacher => teachers::ask_user(screen).await,
        AdminNav::TeacherCourses(id, page) => teachers::show_courses(screen, id, page).await,
        AdminNav::Groups(page) => structure::show_groups(screen, page).await,
        AdminNav::Group(id) => structure::show_group(screen, id).await,
        AdminNav::NewGroup => structure::ask_group_code(screen).await,
        AdminNav::RenameGroup(id) => structure::ask_group_rename(screen, id).await,
        AdminNav::GroupYear(id) => structure::choose_group_year(screen, id).await,
        AdminNav::Subjects(page) => structure::show_subjects(screen, page).await,
        AdminNav::Subject(id) => structure::show_subject(screen, id).await,
        AdminNav::NewSubject => structure::ask_subject_name(screen).await,
        AdminNav::RenameSubject(id) => structure::ask_subject_rename(screen, id).await,
        AdminNav::Courses(page) => lifecycle::show_courses(screen, false, page).await,
        AdminNav::ClosedCourses(page) => lifecycle::show_courses(screen, true, page).await,
        AdminNav::Course(id) => lifecycle::show_course(screen, id).await,
        AdminNav::NewCourse(page) => lifecycle::choose_subject(screen, page).await,
        AdminNav::AddCourseGroup(id, page) => {
            lifecycle::choose_group_to_add(screen, id, page).await
        }
        AdminNav::CloseCourse(id) => lifecycle::ask_close(screen, id).await,
    }
}

pub(super) async fn handle_screen(
    screen: ActorChat<'_>,
    message_id: MessageId,
    action: AdminScreen,
    state: &BotState,
) -> HandlerResult {
    match action {
        AdminScreen::SearchPage(page) => {
            students::show_search_page(screen, message_id, page, state).await
        }
        AdminScreen::ToggleTeacherCourse(course) => {
            teachers::toggle_course(screen, message_id, course, state).await
        }
        AdminScreen::AssignSelf => lifecycle::assign_self(screen, message_id, state).await,
        AdminScreen::Reopen => lifecycle::reopen(screen, message_id, state).await,
    }
}

pub(super) async fn handle_step(
    screen: ActorChat<'_>,
    message_id: MessageId,
    step: AdminStep,
    state: BotState,
) -> HandlerResult {
    let Some(flow) = state.admin_choice(message_id) else {
        return screen.stale().await;
    };
    match (step, flow) {
        (AdminStep::StudentGroup(group_id), AdminFlow::ChoosingStudentGroup { student_id, .. }) => {
            students::choose_group(screen, message_id, student_id, group_id).await
        }
        (
            AdminStep::StudentCourse(course_id),
            AdminFlow::ChoosingStudentCourses {
                student_id,
                group_id,
                selected,
                ..
            },
        ) => {
            students::toggle_course(
                screen, message_id, student_id, group_id, selected, course_id,
            )
            .await
        }
        (
            AdminStep::StudentCoursesDone,
            AdminFlow::ChoosingStudentCourses {
                student_id,
                group_id,
                selected,
                ..
            },
        ) => {
            students::finish_group_change(screen, message_id, student_id, group_id, selected).await
        }
        (
            AdminStep::ConfirmUnenroll,
            AdminFlow::ConfirmingUnenroll {
                student_id,
                course_id,
                ..
            },
        ) => students::confirm_unenroll(screen, message_id, student_id, course_id).await,
        (
            AdminStep::KeepSuggestedName,
            AdminFlow::AwaitingTeacherName {
                telegram_user_id,
                suggested: Some(name),
                ..
            },
        ) => teachers::register(screen, Some(message_id), telegram_user_id, name).await,
        (AdminStep::Level(level), AdminFlow::ChoosingGroupLevel { code, .. }) => {
            structure::choose_level(screen, message_id, code, level).await
        }
        (AdminStep::Year(year), AdminFlow::ChoosingGroupYear { code, level, .. }) => {
            structure::create_group(screen, message_id, code, level, year).await
        }
        (AdminStep::Year(year), AdminFlow::ChoosingGroupNewYear { group_id, .. }) => {
            structure::change_group_year(screen, message_id, group_id, year).await
        }
        (AdminStep::CourseSubject(subject_id), AdminFlow::ChoosingCourseSubject { .. }) => {
            lifecycle::start_course_groups(screen, message_id, subject_id).await
        }
        (
            AdminStep::CourseGroup(group_id),
            AdminFlow::ChoosingCourseGroups {
                subject_id,
                selected,
                page,
                ..
            },
        ) => {
            lifecycle::toggle_course_group(screen, message_id, subject_id, selected, page, group_id)
                .await
        }
        (
            AdminStep::ListPage(page),
            AdminFlow::ChoosingCourseGroups {
                subject_id,
                selected,
                ..
            },
        ) => lifecycle::show_course_groups(screen, message_id, subject_id, selected, page).await,
        (
            AdminStep::CourseGroupsDone,
            AdminFlow::ChoosingCourseGroups {
                subject_id,
                selected,
                ..
            },
        ) => lifecycle::finish_course_groups(screen, message_id, subject_id, selected).await,
        (
            AdminStep::CourseTeacher(teacher_id),
            AdminFlow::ChoosingCourseTeachers {
                subject_id,
                group_ids,
                selected,
                page,
                ..
            },
        ) => {
            let draft = lifecycle::CourseDraft {
                subject_id,
                group_ids,
                selected,
            };
            lifecycle::toggle_course_teacher(screen, message_id, draft, page, teacher_id).await
        }
        (
            AdminStep::ListPage(page),
            AdminFlow::ChoosingCourseTeachers {
                subject_id,
                group_ids,
                selected,
                ..
            },
        ) => {
            let draft = lifecycle::CourseDraft {
                subject_id,
                group_ids,
                selected,
            };
            lifecycle::show_course_teachers(screen, message_id, draft, page).await
        }
        (
            AdminStep::CourseTeachersDone,
            AdminFlow::ChoosingCourseTeachers {
                subject_id,
                group_ids,
                selected,
                ..
            },
        ) => {
            let draft = lifecycle::CourseDraft {
                subject_id,
                group_ids,
                selected,
            };
            lifecycle::create_course(screen, message_id, draft).await
        }
        (AdminStep::AddGroup(group_id), AdminFlow::ChoosingGroupToAdd { course_id, .. }) => {
            lifecycle::add_group(screen, message_id, course_id, group_id).await
        }
        (
            AdminStep::ConfirmClose,
            AdminFlow::ConfirmingCourseClose {
                course_id,
                published,
                pending,
                ..
            },
        ) => lifecycle::confirm_close(screen, message_id, course_id, published, pending).await,
        _ => screen.stale().await,
    }
}

pub(super) async fn submit_text(step: TextStep<'_>, flow: AdminFlow) -> HandlerResult {
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    let Some(text) = step.message().text() else {
        return send_hint(screen, flow.button_only()).await;
    };
    let text = text.to_owned();
    match flow {
        AdminFlow::AwaitingStudentQuery => students::submit_query(screen, text).await,
        AdminFlow::AwaitingStudentName { student_id } => {
            students::submit_name(screen, student_id, text).await
        }
        AdminFlow::AwaitingTeacherUser => teachers::submit_user_id(screen, &text).await,
        AdminFlow::AwaitingTeacherName {
            telegram_user_id,
            message_id,
            ..
        } => teachers::register(screen, message_id, telegram_user_id, text).await,
        AdminFlow::AwaitingGroupCode => structure::submit_group_code(screen, text).await,
        AdminFlow::AwaitingGroupRename { group_id } => {
            structure::submit_group_rename(screen, group_id, text).await
        }
        AdminFlow::AwaitingSubjectName => structure::submit_subject_name(screen, text).await,
        AdminFlow::AwaitingSubjectRename { subject_id } => {
            structure::submit_subject_rename(screen, subject_id, text).await
        }
        flow => send_hint(screen, flow.button_only()).await,
    }
}

pub(super) async fn users_shared(
    step: TextStep<'_>,
    shared: &UsersShared,
    state: &BotState,
) -> HandlerResult {
    if !matches!(
        state,
        BotState::Flow(FlowState::Admin(AdminFlow::AwaitingTeacherUser))
    ) {
        return Ok(());
    }
    let Some(screen) = step.resolve().await? else {
        return Ok(());
    };
    teachers::user_shared(screen, shared).await
}

async fn send_hint(screen: ActorChat<'_>, button_only: bool) -> HandlerResult {
    let hint = if button_only {
        texts::CHOOSE_BUTTON
    } else {
        texts::ADMIN_TEXT_REQUIRED
    };
    screen.bot().send_message(screen.chat_id(), hint).await?;
    Ok(())
}

async fn show_home(screen: ActorChat<'_>) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    reset_dialogue(screen.dialogue()).await?;
    send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_HOME,
        admin_home_keyboard(),
    )
    .await?;
    Ok(())
}

async fn set_flow(screen: ActorChat<'_>, flow: AdminFlow) -> HandlerResult {
    screen
        .dialogue()
        .update(BotState::Flow(FlowState::Admin(flow)))
        .await?;
    Ok(())
}

async fn finish(screen: ActorChat<'_>, text: &str) -> HandlerResult {
    reset_dialogue(screen.dialogue()).await?;
    screen
        .bot()
        .send_message(screen.chat_id(), text)
        .reply_markup(admin_keyboard())
        .await?;
    Ok(())
}

async fn end_on_refusal<T>(
    screen: ActorChat<'_>,
    result: AppResult<T>,
) -> HandlerResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.code().is_server_failure() => Err(error.into()),
        Err(error) => {
            finish(screen, app_error_message(error.code())).await?;
            Ok(None)
        }
    }
}
