use chrono::{DateTime, Utc};
use teloxide::types::MessageId;

use super::callbacks::MakeupFormAction;
use super::form::{Form, InputMode};
use crate::app::{Actor, ConfigureMakeup, MakeupDecision, MakeupSettings, UndecidedAssignment};
use crate::domain::{
    AssignmentId, CourseMakeup, DomainError, Grading, MakeupTerms, TeachingCourseId,
};

#[derive(Clone, Debug)]
pub(super) struct ScaleQuestion {
    pub grading: Grading,
    pub assignments: Vec<(AssignmentId, String)>,
    pub answer: Option<MakeupTerms>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MakeupStep {
    Date,
    Attempts,
    Scale(usize),
    Summary,
}

#[derive(Clone, Debug)]
pub(super) struct MakeupForm {
    pub course_id: TeachingCourseId,
    pub saved: Option<CourseMakeup>,
    pub active: bool,
    pub until: Option<DateTime<Utc>>,
    pub attempts: Option<u16>,
    pub disabling: bool,
    pub scales: Vec<ScaleQuestion>,
    pub step: MakeupStep,
    pub mode: InputMode,
    pub message_id: Option<MessageId>,
    pub return_to_summary: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum MakeupInputError {
    UnexpectedStep,
    Domain(DomainError),
}

impl MakeupForm {
    pub fn new(
        course_id: TeachingCourseId,
        saved: Option<CourseMakeup>,
        undecided: Vec<UndecidedAssignment>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut scales: Vec<ScaleQuestion> = Vec::new();
        for assignment in undecided {
            let entry = (assignment.assignment_id, assignment.title);
            match scales
                .iter_mut()
                .find(|scale| scale.grading == assignment.grading)
            {
                Some(scale) => scale.assignments.push(entry),
                None => scales.push(ScaleQuestion {
                    grading: assignment.grading,
                    assignments: vec![entry],
                    answer: None,
                }),
            }
        }
        let active = saved.filter(|makeup| makeup.is_active_at(now));
        let mut form = Self {
            course_id,
            saved,
            active: active.is_some(),
            until: active.map(CourseMakeup::until),
            attempts: active.map(CourseMakeup::attempts),
            disabling: false,
            scales,
            step: MakeupStep::Date,
            mode: InputMode::Buttons,
            message_id: None,
            return_to_summary: false,
        };
        if form.active {
            form.step = form.after_settings();
        }
        form
    }

    pub fn ended(&self) -> Option<CourseMakeup> {
        self.saved.filter(|_| !self.active)
    }

    pub fn notice_end(&mut self, now: DateTime<Utc>) -> bool {
        let ended = self.active
            && !self.disabling
            && self.saved.is_some_and(|makeup| !makeup.is_active_at(now));
        if ended {
            self.active = false;
        }
        ended
    }

    pub fn current_scale(&self) -> Option<&ScaleQuestion> {
        match self.step {
            MakeupStep::Scale(index) => self.scales.get(index),
            MakeupStep::Date | MakeupStep::Attempts | MakeupStep::Summary => None,
        }
    }

    pub fn go(&mut self, step: MakeupStep) {
        self.step = step;
        self.mode = InputMode::Buttons;
        if step == MakeupStep::Summary {
            self.return_to_summary = false;
        }
    }

    pub fn apply_attempts(&mut self, attempts: u16) -> Result<(), MakeupInputError> {
        if self.step != MakeupStep::Attempts {
            return Err(MakeupInputError::UnexpectedStep);
        }
        CourseMakeup::ensure_attempts(attempts).map_err(MakeupInputError::Domain)?;
        self.attempts = Some(attempts);
        self.advance();
        Ok(())
    }

    pub fn answer(&mut self, terms: MakeupTerms) -> Result<(), MakeupInputError> {
        let MakeupStep::Scale(index) = self.step else {
            return Err(MakeupInputError::UnexpectedStep);
        };
        let scale = self
            .scales
            .get_mut(index)
            .ok_or(MakeupInputError::UnexpectedStep)?;
        terms
            .ensure_fits(scale.grading)
            .map_err(MakeupInputError::Domain)?;
        scale.answer = Some(terms);
        self.advance();
        Ok(())
    }

    pub fn edit(&mut self, action: MakeupFormAction) -> bool {
        if self.step != MakeupStep::Summary {
            return false;
        }
        match action {
            MakeupFormAction::Date if !self.disabling => self.edit_from_summary(MakeupStep::Date),
            MakeupFormAction::Attempts if !self.disabling => {
                self.edit_from_summary(MakeupStep::Attempts)
            }
            MakeupFormAction::Disable if self.active => self.disabling = !self.disabling,
            _ => return false,
        }
        true
    }

    pub fn command(&self, actor: Actor, now: DateTime<Utc>) -> Option<ConfigureMakeup> {
        let settings = if self.disabling {
            None
        } else {
            Some(MakeupSettings {
                until: self.until?,
                attempts: self.attempts?,
            })
        };

        Some(ConfigureMakeup {
            course_id: self.course_id,
            actor,
            expected: self.saved,
            settings,
            decisions: self
                .scales
                .iter()
                .filter_map(|scale| {
                    Some(MakeupDecision {
                        grading: scale.grading,
                        assignment_ids: scale.assignments.iter().map(|(id, _)| *id).collect(),
                        terms: scale.answer?,
                    })
                })
                .collect(),
            now,
        })
    }

    fn edit_from_summary(&mut self, step: MakeupStep) {
        self.go(step);
        self.return_to_summary = true;
    }

    fn after_settings(&self) -> MakeupStep {
        if self.scales.is_empty() {
            MakeupStep::Summary
        } else {
            MakeupStep::Scale(0)
        }
    }

    fn previous(&self) -> Option<MakeupStep> {
        if self.return_to_summary && self.step != MakeupStep::Summary {
            return Some(MakeupStep::Summary);
        }
        match self.step {
            MakeupStep::Date | MakeupStep::Summary => None,
            MakeupStep::Attempts => Some(MakeupStep::Date),
            MakeupStep::Scale(0) => (!self.active).then_some(MakeupStep::Attempts),
            MakeupStep::Scale(index) => Some(MakeupStep::Scale(index - 1)),
        }
    }

    fn advance(&mut self) {
        let step = if self.return_to_summary {
            MakeupStep::Summary
        } else {
            match self.step {
                MakeupStep::Date => MakeupStep::Attempts,
                MakeupStep::Attempts => self.after_settings(),
                MakeupStep::Scale(index) if index + 1 < self.scales.len() => {
                    MakeupStep::Scale(index + 1)
                }
                MakeupStep::Scale(_) | MakeupStep::Summary => MakeupStep::Summary,
            }
        };
        self.go(step);
    }
}

impl Form for MakeupForm {
    type Step = MakeupStep;
    type InputError = MakeupInputError;

    fn step(&self) -> MakeupStep {
        self.step
    }

    fn mode(&self) -> InputMode {
        self.mode
    }

    fn set_mode(&mut self, mode: InputMode) {
        self.mode = mode;
    }

    fn message_id(&self) -> Option<MessageId> {
        self.message_id
    }

    fn message_id_mut(&mut self) -> &mut Option<MessageId> {
        &mut self.message_id
    }

    fn can_go_back(&self) -> bool {
        self.previous().is_some()
    }

    fn back(&mut self) -> bool {
        if self.mode != InputMode::Buttons {
            self.mode = InputMode::Buttons;
            return true;
        }
        let Some(step) = self.previous() else {
            return false;
        };
        self.go(step);
        true
    }

    fn accepts_text(&self) -> bool {
        match self.step {
            MakeupStep::Date | MakeupStep::Attempts => true,
            MakeupStep::Scale(_) => self
                .current_scale()
                .is_some_and(|scale| scale.grading.is_scored()),
            MakeupStep::Summary => false,
        }
    }

    fn apply_date(
        &mut self,
        until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), MakeupInputError> {
        if self.step != MakeupStep::Date {
            return Err(MakeupInputError::UnexpectedStep);
        }
        CourseMakeup::ensure_until(until, now).map_err(MakeupInputError::Domain)?;
        self.until = Some(until);
        self.advance();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::{MAX_MAKEUP_ATTEMPTS, UserId};
    use crate::test_support::t0;

    fn undecided(title: &str, grading: Grading) -> UndecidedAssignment {
        UndecidedAssignment {
            assignment_id: AssignmentId::new(),
            title: title.to_owned(),
            grading,
        }
    }

    fn two_scales() -> Vec<UndecidedAssignment> {
        let ten = Grading::scored(10).unwrap();
        vec![
            undecided("Lab 1", ten),
            undecided("Lab 2", ten),
            undecided("Quiz", Grading::PassFail),
        ]
    }

    fn enabled() -> CourseMakeup {
        CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap()
    }

    #[test]
    fn enabling_asks_the_date_the_pool_and_each_scale_before_the_summary() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), None, two_scales(), t0());
        assert_eq!(form.scales.len(), 2);
        assert_eq!(form.scales[0].assignments.len(), 2);
        assert_eq!(form.step, MakeupStep::Date);

        form.apply_date(t0() + Duration::days(30), t0()).unwrap();
        assert_eq!(form.step, MakeupStep::Attempts);
        form.apply_attempts(3).unwrap();
        assert_eq!(form.step, MakeupStep::Scale(0));
        form.answer(MakeupTerms::scored(6, form.scales[0].grading).unwrap())
            .unwrap();
        assert_eq!(form.step, MakeupStep::Scale(1));
        form.answer(MakeupTerms::PassFail).unwrap();
        assert_eq!(form.step, MakeupStep::Summary);

        let command = form.command(Actor::new(UserId::new()), t0()).unwrap();
        assert_eq!(command.expected, None);
        assert_eq!(command.settings.map(|settings| settings.attempts), Some(3));
        assert_eq!(command.decisions.len(), 2);
        assert_eq!(command.decisions[0].assignment_ids.len(), 2);
    }

    #[test]
    fn an_enabled_course_opens_at_its_questions_or_its_summary() {
        let with_questions =
            MakeupForm::new(TeachingCourseId::new(), Some(enabled()), two_scales(), t0());
        assert_eq!(with_questions.step, MakeupStep::Scale(0));
        assert_eq!(with_questions.attempts, Some(3));

        let decided = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());
        assert_eq!(decided.step, MakeupStep::Summary);
        assert_eq!(decided.until, Some(enabled().until()));
    }

    #[test]
    fn an_ended_makeup_is_configured_again_like_a_disabled_one() {
        let ended_at = enabled().until();
        let mut form = MakeupForm::new(
            TeachingCourseId::new(),
            Some(enabled()),
            two_scales(),
            ended_at,
        );

        assert!(!form.active);
        assert_eq!(form.ended(), Some(enabled()));
        assert_eq!(form.step, MakeupStep::Date);
        assert_eq!((form.until, form.attempts), (None, None));
        form.apply_date(ended_at + Duration::days(30), ended_at)
            .unwrap();
        form.apply_attempts(2).unwrap();
        assert!(form.back());
        assert_eq!(form.step, MakeupStep::Attempts);
        form.apply_attempts(2).unwrap();
        form.answer(MakeupTerms::NotOffered).unwrap();
        form.answer(MakeupTerms::PassFail).unwrap();
        assert!(!form.edit(MakeupFormAction::Disable));

        let command = form.command(Actor::new(UserId::new()), ended_at).unwrap();
        assert_eq!(command.expected, Some(enabled()));
        assert_eq!(command.settings.map(|settings| settings.attempts), Some(2));
    }

    #[test]
    fn an_end_reached_while_editing_turns_the_save_into_enabling() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());
        assert!(!form.notice_end(enabled().until() - Duration::seconds(1)));
        assert!(form.active);

        assert!(form.notice_end(enabled().until()));
        assert!(!form.active);
        assert_eq!(form.ended(), Some(enabled()));
        assert!(!form.edit(MakeupFormAction::Disable));
        assert!(!form.notice_end(enabled().until()));
    }

    #[test]
    fn disabling_a_makeup_that_ended_meanwhile_is_still_saved_as_disabling() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());
        assert!(form.edit(MakeupFormAction::Disable));

        assert!(!form.notice_end(enabled().until()));
        assert!(form.disabling);
    }

    #[test]
    fn cancelling_a_scale_question_returns_to_the_pool_with_the_entered_values() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), None, two_scales(), t0());
        form.apply_date(t0() + Duration::days(30), t0()).unwrap();
        form.apply_attempts(3).unwrap();

        assert!(form.back());
        assert_eq!(form.step, MakeupStep::Attempts);
        assert_eq!(form.attempts, Some(3));
        assert!(form.back());
        assert_eq!(form.step, MakeupStep::Date);
        assert!(!form.back());

        let mut enabled_form =
            MakeupForm::new(TeachingCourseId::new(), Some(enabled()), two_scales(), t0());
        assert!(!enabled_form.back());
    }

    #[test]
    fn an_edit_from_the_summary_returns_to_the_summary_without_asking_again() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());

        assert!(form.edit(MakeupFormAction::Attempts));
        assert_eq!(form.step, MakeupStep::Attempts);
        form.apply_attempts(4).unwrap();

        assert_eq!(form.step, MakeupStep::Summary);
        assert_eq!(form.attempts, Some(4));
        assert!(form.edit(MakeupFormAction::Date));
        assert!(form.back());
        assert_eq!(form.step, MakeupStep::Summary);
        assert!(!form.back());
    }

    #[test]
    fn the_back_button_leaves_the_calendar_for_the_summary_unlike_cancel() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());
        assert!(form.edit(MakeupFormAction::Date));
        form.mode = InputMode::Calendar(t0().date_naive());
        let mut cancelled = form.clone();

        assert!(form.step_back());
        assert_eq!(form.step, MakeupStep::Summary);
        assert!(cancelled.back());
        assert_eq!(cancelled.step, MakeupStep::Date);
        assert_eq!(cancelled.mode, InputMode::Buttons);
    }

    #[test]
    fn a_pool_out_of_range_keeps_the_step_and_reports_the_limit() {
        let mut form = MakeupForm::new(TeachingCourseId::new(), None, Vec::new(), t0());
        form.apply_date(t0() + Duration::days(30), t0()).unwrap();

        for attempts in [0, MAX_MAKEUP_ATTEMPTS + 1] {
            assert_eq!(
                form.apply_attempts(attempts),
                Err(MakeupInputError::Domain(
                    DomainError::InvalidMakeupAttempts {
                        value: attempts,
                        max: MAX_MAKEUP_ATTEMPTS,
                    }
                ))
            );
            assert_eq!(form.step, MakeupStep::Attempts);
        }
    }

    #[test]
    fn disabling_is_offered_only_for_an_enabled_course_and_saved_as_no_settings() {
        let mut new = MakeupForm::new(TeachingCourseId::new(), None, Vec::new(), t0());
        new.go(MakeupStep::Summary);
        assert!(!new.edit(MakeupFormAction::Disable));

        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(enabled()), Vec::new(), t0());
        assert!(form.edit(MakeupFormAction::Disable));
        assert!(!form.edit(MakeupFormAction::Date));
        let command = form.command(Actor::new(UserId::new()), t0()).unwrap();
        assert_eq!(command.settings, None);
        assert_eq!(command.expected, Some(enabled()));
        assert!(form.edit(MakeupFormAction::Disable));
        assert!(!form.disabling);
    }

    #[test]
    fn an_answer_must_fit_the_scale_of_its_question() {
        let mut form =
            MakeupForm::new(TeachingCourseId::new(), Some(enabled()), two_scales(), t0());

        assert_eq!(
            form.answer(MakeupTerms::PassFail),
            Err(MakeupInputError::Domain(
                DomainError::MakeupTermsDoNotFitGrading
            ))
        );
        assert_eq!(form.step, MakeupStep::Scale(0));
        form.answer(MakeupTerms::NotOffered).unwrap();
        assert_eq!(form.step, MakeupStep::Scale(1));
    }
}
