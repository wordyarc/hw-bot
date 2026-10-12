use teloxide::types::ChatId;

use crate::bot::context::Screen;
use crate::bot::state::ActiveForm;
use crate::bot::{Bot, HandlerResult};

mod attempts;
mod courses;
mod creation;
mod deadline;
mod form;
mod gradebook;
mod invites;
mod makeup;
mod roster;
mod terms;

pub(super) use attempts::handle_extra_attempts;
pub(super) use attempts::select_student;
pub(super) use attempts::show_students_page;
pub(super) use attempts::submit_attempt_amount;
pub(super) use courses::show_assignment;
pub(super) use courses::{archive_assignment, publish_assignment};
pub(super) use courses::{show_course_assignments_page, show_courses_page};
pub(super) use creation::handle_grading;
pub(super) use creation::handle_max_attempts;
pub(super) use creation::submit_body;
pub(super) use creation::submit_grading_text;
pub(super) use creation::submit_max_attempts;
pub(super) use creation::submit_title;
pub(super) use creation::{finish_assignment_groups, toggle_assignment_group};
pub(super) use deadline::handle_deadline;
pub(super) use gradebook::{show_gradebook_groups, show_gradebook_page, show_student_grades_page};
pub(super) use invites::show_course_invite;
pub(super) use invites::{ask_invite_rotation, cancel_invite_rotation, confirm_invite_rotation};
pub(super) use makeup::{
    handle_answer as handle_makeup_answer, handle_attempts as handle_makeup_attempts,
    handle_form as handle_makeup_form, open as open_makeup, submit_text as submit_makeup_text,
};
pub(super) use roster::show_roster_page;
pub(super) use terms::{
    begin_terms_edit, confirm_exit, handle_amount as handle_terms_amount, handle_duration_periods,
    handle_late_policy, handle_makeup, handle_period, handle_terms, submit_terms,
};

pub(super) async fn abandon_form(
    bot: &Bot,
    chat_id: ChatId,
    form: ActiveForm<'_>,
) -> HandlerResult {
    match form {
        ActiveForm::Terms(form) => form::abandon(bot, chat_id, form).await,
        ActiveForm::Makeup(form) => form::abandon(bot, chat_id, form).await,
    }
}

pub(super) async fn form_back(screen: Screen<'_>, form: ActiveForm<'_>) -> HandlerResult {
    match form {
        ActiveForm::Terms(form) => form::back(screen, form.clone()).await,
        ActiveForm::Makeup(form) => form::back(screen, form.clone()).await,
    }
}
