use teloxide::types::MessageId;

use super::{end_on_refusal, finish, set_flow};
use crate::app::{AppError, CreateStudentGroup};
use crate::bot::HandlerResult;
use crate::bot::callbacks::AdminNav;
use crate::bot::context::ActorChat;
use crate::bot::handlers::reset_dialogue;
use crate::bot::handlers::teacher::detach;
use crate::bot::state::AdminFlow;
use crate::bot::texts;
use crate::bot::views::{
    admin_group_keyboard, admin_groups_keyboard, admin_levels_keyboard, admin_subject_keyboard,
    admin_subjects_keyboard, admin_years_keyboard, send_screen,
};
use crate::domain::{EducationLevel, StudentGroup, StudentGroupId, StudyProgram, SubjectId};
use crate::pagination::PageRequest;

pub(super) async fn show_groups(screen: ActorChat<'_>, page: u32) -> HandlerResult {
    let groups = screen
        .services()
        .admin
        .list_groups(screen.actor(), PageRequest::new(page))
        .await?;
    let text = if groups.items.is_empty() {
        texts::ADMIN_NO_GROUPS
    } else {
        texts::ADMIN_GROUPS
    };
    send_screen(
        screen.bot(),
        screen.chat_id(),
        text,
        admin_groups_keyboard(&groups),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn show_group(screen: ActorChat<'_>, group_id: StudentGroupId) -> HandlerResult {
    let group = screen
        .services()
        .admin
        .find_group(screen.actor(), group_id)
        .await?;
    let text = texts::admin_group_card(
        group.code().as_str(),
        group.program().education_level(),
        group.program().year(),
    );
    send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_group_keyboard(group_id),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn ask_group_code(screen: ActorChat<'_>) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    set_flow(screen, AdminFlow::AwaitingGroupCode).await?;
    screen.ask(texts::ADMIN_GROUP_CODE_PROMPT).await
}

pub(super) async fn submit_group_code(screen: ActorChat<'_>, code: String) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    let program = StudyProgram::new(EducationLevel::Bachelor, 1).map_err(AppError::from)?;
    let code = StudentGroup::new(StudentGroupId::new(), code, program)
        .map_err(AppError::from)?
        .code()
        .as_str()
        .to_owned();
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_CHOOSE_LEVEL,
        admin_levels_keyboard(),
    )
    .await?;
    set_flow(screen, AdminFlow::ChoosingGroupLevel { message_id, code }).await
}

pub(super) async fn choose_level(
    screen: ActorChat<'_>,
    message_id: MessageId,
    code: String,
    level: EducationLevel,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_CHOOSE_YEAR,
        admin_years_keyboard(level, AdminNav::Groups(0)),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingGroupYear {
            message_id,
            code,
            level,
        },
    )
    .await
}

pub(super) async fn create_group(
    screen: ActorChat<'_>,
    message_id: MessageId,
    code: String,
    education_level: EducationLevel,
    study_year: u8,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let created = screen
        .services()
        .admin
        .create_group(CreateStudentGroup {
            actor: screen.actor(),
            code,
            education_level,
            study_year,
        })
        .await;
    let Some(group) = end_on_refusal(screen, created).await? else {
        return show_groups(screen, 0).await;
    };
    finish(screen, texts::ADMIN_GROUP_CREATED).await?;
    show_group(screen, group.id()).await
}

pub(super) async fn ask_group_rename(
    screen: ActorChat<'_>,
    group_id: StudentGroupId,
) -> HandlerResult {
    screen
        .services()
        .admin
        .find_group(screen.actor(), group_id)
        .await?;
    set_flow(screen, AdminFlow::AwaitingGroupRename { group_id }).await?;
    screen.ask(texts::ADMIN_GROUP_RENAME_PROMPT).await
}

pub(super) async fn submit_group_rename(
    screen: ActorChat<'_>,
    group_id: StudentGroupId,
    code: String,
) -> HandlerResult {
    screen
        .services()
        .admin
        .rename_group(screen.actor(), group_id, code)
        .await?;
    finish(screen, texts::ADMIN_GROUP_RENAMED).await?;
    show_group(screen, group_id).await
}

pub(super) async fn choose_group_year(
    screen: ActorChat<'_>,
    group_id: StudentGroupId,
) -> HandlerResult {
    let group = screen
        .services()
        .admin
        .find_group(screen.actor(), group_id)
        .await?;
    let message_id = send_screen(
        screen.bot(),
        screen.chat_id(),
        texts::ADMIN_CHOOSE_YEAR,
        admin_years_keyboard(group.program().education_level(), AdminNav::Group(group_id)),
    )
    .await?;
    set_flow(
        screen,
        AdminFlow::ChoosingGroupNewYear {
            message_id,
            group_id,
        },
    )
    .await
}

pub(super) async fn change_group_year(
    screen: ActorChat<'_>,
    message_id: MessageId,
    group_id: StudentGroupId,
    year: u8,
) -> HandlerResult {
    detach(screen.bot(), screen.chat_id(), Some(message_id)).await?;
    let changed = screen
        .services()
        .admin
        .change_group_year(screen.actor(), group_id, year)
        .await;
    if end_on_refusal(screen, changed).await?.is_some() {
        finish(screen, texts::ADMIN_GROUP_YEAR_CHANGED).await?;
    }
    show_group(screen, group_id).await
}

pub(super) async fn show_subjects(screen: ActorChat<'_>, page: u32) -> HandlerResult {
    let subjects = screen
        .services()
        .admin
        .list_subjects(screen.actor(), PageRequest::new(page))
        .await?;
    let text = if subjects.items.is_empty() {
        texts::ADMIN_NO_SUBJECTS
    } else {
        texts::ADMIN_SUBJECTS
    };
    send_screen(
        screen.bot(),
        screen.chat_id(),
        text,
        admin_subjects_keyboard(&subjects),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn show_subject(screen: ActorChat<'_>, subject_id: SubjectId) -> HandlerResult {
    let stored = screen
        .services()
        .admin
        .find_subject(screen.actor(), subject_id)
        .await?;
    let text = texts::admin_subject_card(stored.subject.name().as_str(), stored.sandbox);
    send_screen(
        screen.bot(),
        screen.chat_id(),
        &text,
        admin_subject_keyboard(subject_id),
    )
    .await?;
    reset_dialogue(screen.dialogue()).await
}

pub(super) async fn ask_subject_name(screen: ActorChat<'_>) -> HandlerResult {
    screen.services().admin.ensure_admin(screen.actor()).await?;
    set_flow(screen, AdminFlow::AwaitingSubjectName).await?;
    screen.ask(texts::ADMIN_SUBJECT_NAME_PROMPT).await
}

pub(super) async fn submit_subject_name(screen: ActorChat<'_>, name: String) -> HandlerResult {
    let subject = screen
        .services()
        .admin
        .create_subject(screen.actor(), name)
        .await?;
    finish(screen, texts::ADMIN_SUBJECT_CREATED).await?;
    show_subject(screen, subject.id()).await
}

pub(super) async fn ask_subject_rename(
    screen: ActorChat<'_>,
    subject_id: SubjectId,
) -> HandlerResult {
    screen
        .services()
        .admin
        .find_subject(screen.actor(), subject_id)
        .await?;
    set_flow(screen, AdminFlow::AwaitingSubjectRename { subject_id }).await?;
    screen.ask(texts::ADMIN_SUBJECT_RENAME_PROMPT).await
}

pub(super) async fn submit_subject_rename(
    screen: ActorChat<'_>,
    subject_id: SubjectId,
    name: String,
) -> HandlerResult {
    screen
        .services()
        .admin
        .rename_subject(screen.actor(), subject_id, name)
        .await?;
    finish(screen, texts::ADMIN_SUBJECT_RENAMED).await?;
    show_subject(screen, subject_id).await
}
