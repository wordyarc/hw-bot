//! Преподавательские флоу. Диспетчер зовёт их через реэкспорты ниже, поэтому разрез по флоу
//! не меняет его вида.

mod attempts;
mod courses;
mod creation;
mod deadline;
mod gradebook;
mod invites;

pub(super) use attempts::handle_extra_attempts;
pub(super) use attempts::select_student;
pub(super) use attempts::show_students;
pub(super) use attempts::submit_attempt_amount;
pub(super) use courses::handle_screen_action;
pub(super) use courses::show_assignment;
pub(super) use courses::show_course_assignments;
pub(super) use courses::show_courses;
pub(super) use creation::handle_grading;
pub(super) use creation::handle_late_policy;
pub(super) use creation::handle_max_attempts;
pub(super) use creation::submit_body;
pub(super) use creation::submit_grading_text;
pub(super) use creation::submit_late_policy_text;
pub(super) use creation::submit_max_attempts;
pub(super) use creation::submit_title;
pub(super) use deadline::handle_deadline;
pub(super) use deadline::submit_deadline_text;
pub(super) use gradebook::show_gradebook;
pub(super) use invites::handle_invite_rotation;
pub(super) use invites::show_group_invite;
