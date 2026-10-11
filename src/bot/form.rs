use std::fmt::Debug;

use chrono::{DateTime, NaiveDate, Utc};
use teloxide::types::MessageId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputMode {
    Buttons,
    Manual,
    Calendar(NaiveDate),
}

pub(super) trait Form {
    type Step: Copy + Debug;
    type InputError;

    fn step(&self) -> Self::Step;
    fn mode(&self) -> InputMode;
    fn set_mode(&mut self, mode: InputMode);
    fn message_id(&self) -> Option<MessageId>;
    fn message_id_mut(&mut self) -> &mut Option<MessageId>;
    fn can_go_back(&self) -> bool;
    fn back(&mut self) -> bool;
    fn accepts_text(&self) -> bool;
    fn apply_date(&mut self, at: DateTime<Utc>, now: DateTime<Utc>)
    -> Result<(), Self::InputError>;

    fn step_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        if matches!(self.mode(), InputMode::Calendar(_)) {
            self.set_mode(InputMode::Buttons);
        }
        self.back()
    }
}
