use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use teloxide::types::MessageId;

use super::callbacks::TermsAction;
use super::form::{Form, InputMode};
use crate::app::MakeupQuestion;
use crate::domain::{
    Assignment, AssignmentId, DeductedFrom, DomainError, DomainResult, Grading, LatePolicy,
    MAX_LATE_PERIOD_DAYS, MAX_LATE_PERIODS, MAX_PENALTY_PER_PERIOD, MakeupTerms, StudentGroup,
    StudentGroupId, TeachingCourseId,
};

const DEFAULT_PERIOD_DAYS: u16 = 7;

#[derive(Clone)]
pub(super) struct AssignmentText {
    pub course_id: TeachingCourseId,
    pub title: String,
    pub body: String,
}

#[derive(Clone)]
pub(super) enum IssuedGroups {
    Single(StudentGroupId),
    Chosen(Vec<StudentGroup>),
}

impl IssuedGroups {
    pub fn ids(&self) -> BTreeSet<StudentGroupId> {
        match self {
            Self::Single(group_id) => BTreeSet::from([*group_id]),
            Self::Chosen(groups) => groups.iter().map(StudentGroup::id).collect(),
        }
    }

    pub fn listed(&self) -> Option<Vec<&str>> {
        match self {
            Self::Single(_) => None,
            Self::Chosen(groups) => {
                Some(groups.iter().map(|group| group.code().as_str()).collect())
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct DraftAssignment {
    pub text: AssignmentText,
    pub groups: IssuedGroups,
    pub max_attempts: u16,
    pub grading: Grading,
}

impl DraftAssignment {
    pub fn new(text: AssignmentText, groups: IssuedGroups, max_attempts: u16) -> Self {
        Self {
            text,
            groups,
            max_attempts,
            grading: Grading::PassFail,
        }
    }
}

#[derive(Clone)]
pub(super) enum TermsTarget {
    Create(DraftAssignment),
    Edit {
        assignment_id: AssignmentId,
        grading: Grading,
        revision: i64,
        created_at: DateTime<Utc>,
        original_closes_at: DateTime<Utc>,
    },
}

#[derive(Clone)]
pub(super) struct AssignmentTerms {
    pub target: TermsTarget,
    pub deadline_at: Option<DateTime<Utc>>,
    pub deducted_from: Option<DeductedFrom>,
    pub period_days: u16,
    pub penalty_per_period: Option<u16>,
    pub max_periods: Option<u16>,
    pub makeup: Option<MakeupTerms>,
    pub makeup_enabled: bool,
    pub makeup_hint: Option<u16>,
}

/// Условия, по которым можно показать итог и сохранить задание: срок задан, политика опозданий
/// выключена либо собрана целиком, момент закрытия приёма представим.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompleteTerms {
    pub deadline_at: DateTime<Utc>,
    pub late_policy: Option<LatePolicy>,
    pub closes_at: DateTime<Utc>,
}

impl AssignmentTerms {
    pub fn creating(draft: DraftAssignment, question: MakeupQuestion) -> Self {
        Self {
            target: TermsTarget::Create(draft),
            deadline_at: None,
            deducted_from: None,
            period_days: DEFAULT_PERIOD_DAYS,
            penalty_per_period: None,
            max_periods: None,
            makeup: None,
            makeup_enabled: question.enabled,
            makeup_hint: question.hint,
        }
    }

    pub fn editing(assignment: &Assignment, revision: i64, question: MakeupQuestion) -> Self {
        let policy = assignment.late_policy();

        Self {
            target: TermsTarget::Edit {
                assignment_id: assignment.id(),
                grading: assignment.grading(),
                revision,
                created_at: assignment.created_at(),
                original_closes_at: assignment.closes_at(),
            },
            deadline_at: Some(assignment.deadline_at()),
            deducted_from: policy.map(LatePolicy::deducted_from),
            period_days: policy.map_or(DEFAULT_PERIOD_DAYS, LatePolicy::period_days),
            penalty_per_period: policy.map(LatePolicy::penalty_per_period),
            max_periods: policy.map(LatePolicy::max_periods),
            makeup: assignment.makeup(),
            makeup_enabled: question.enabled,
            makeup_hint: question.hint,
        }
    }

    pub fn first_step(&self) -> TermsStep {
        if self.makeup_enabled {
            TermsStep::Makeup
        } else if self.grading().is_scored() {
            TermsStep::Policy
        } else {
            TermsStep::Deadline
        }
    }

    pub fn first_incomplete_step(&self) -> TermsStep {
        let mut step = TermsStep::Deadline;
        while step.is_complete(self) {
            step = step.next(self);
        }
        step
    }

    pub fn complete(&self) -> Option<CompleteTerms> {
        let deadline_at = self.deadline_at?;
        let late_policy = match self.deducted_from {
            Some(deducted_from) => Some(
                LatePolicy::new(
                    self.period_days,
                    self.penalty_per_period?,
                    self.max_periods?,
                    deducted_from,
                )
                .ok()?,
            ),
            None => None,
        };
        let closes_at = Assignment::closing_for(deadline_at, late_policy).ok()?;

        Some(CompleteTerms {
            deadline_at,
            late_policy,
            closes_at,
        })
    }

    /// Все значения введены, но итог по ним построить нельзя: момент закрытия приёма не
    /// представим.
    pub fn cannot_be_summarised(&self) -> bool {
        self.first_incomplete_step() == TermsStep::Summary && self.complete().is_none()
    }

    pub fn preview_policy(&self, max_periods: u16) -> Option<LatePolicy> {
        // Пока штраф не введён, это значение нужно лишь для превью и в сохранённые условия
        // не попадёт.
        const PREVIEW_PENALTY_PER_PERIOD: u16 = 1;
        LatePolicy::new(
            self.period_days,
            self.penalty_per_period
                .unwrap_or(PREVIEW_PENALTY_PER_PERIOD),
            max_periods,
            self.deducted_from?,
        )
        .ok()
    }

    /// Проверяет уже введённую часть условий по правилам нового ввода, включая предел
    /// длительности опоздания.
    pub fn validate(&self) -> DomainResult<()> {
        let Some(deadline) = self.deadline_at else {
            return Ok(());
        };
        if let TermsTarget::Edit { created_at, .. } = self.target
            && deadline <= created_at
        {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }
        match self.preview_policy(self.max_periods.unwrap_or(1)) {
            Some(policy) => policy.validate_horizon(deadline)?,
            None => {
                Assignment::closing_for(deadline, None)?;
            }
        }

        Ok(())
    }

    fn ensure_deadline_ahead(
        &self,
        deadline: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), TermsInputError> {
        if matches!(self.target, TermsTarget::Create(_)) && deadline <= now {
            return Err(TermsInputError::DeadlineInPast);
        }
        Ok(())
    }

    pub fn grading(&self) -> Grading {
        match &self.target {
            TermsTarget::Create(draft) => draft.grading,
            TermsTarget::Edit { grading, .. } => *grading,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TermsStep {
    Makeup,
    Policy,
    Deadline,
    Penalty,
    Period,
    Periods,
    DurationDate,
    DurationConfirm {
        selected: DateTime<Utc>,
        periods: u16,
    },
    Summary,
}

impl TermsStep {
    pub fn next(self, terms: &AssignmentTerms) -> Self {
        match self {
            Self::Makeup if terms.grading().is_scored() => Self::Policy,
            Self::Makeup | Self::Policy => terms.first_incomplete_step(),
            Self::Deadline if terms.deducted_from.is_some() => Self::Penalty,
            Self::Deadline => Self::Summary,
            Self::Period => Self::Penalty,
            Self::Penalty => Self::Periods,
            Self::Periods | Self::DurationConfirm { .. } | Self::Summary => Self::Summary,
            Self::DurationDate => Self::DurationDate,
        }
    }

    pub fn previous(self, terms: &AssignmentTerms) -> Option<Self> {
        let makeup = terms.makeup_enabled.then_some(Self::Makeup);
        match self {
            Self::Makeup => None,
            Self::Policy => makeup,
            Self::Deadline if terms.grading().is_scored() => Some(Self::Policy),
            Self::Deadline => makeup,
            Self::Penalty => Some(Self::Deadline),
            Self::Period | Self::Periods => Some(Self::Penalty),
            Self::DurationDate => Some(Self::Periods),
            Self::DurationConfirm { .. } => Some(Self::DurationDate),
            Self::Summary if matches!(terms.target, TermsTarget::Edit { .. }) => None,
            Self::Summary if terms.deducted_from.is_some() => Some(Self::Periods),
            Self::Summary => Some(Self::Deadline),
        }
    }

    fn is_complete(self, terms: &AssignmentTerms) -> bool {
        match self {
            Self::Deadline => terms.deadline_at.is_some(),
            Self::Penalty => terms.penalty_per_period.is_some(),
            Self::Periods => terms.max_periods.is_some(),
            Self::Makeup
            | Self::Policy
            | Self::Period
            | Self::DurationDate
            | Self::DurationConfirm { .. }
            | Self::Summary => false,
        }
    }

    pub fn summary_edit(action: TermsAction, terms: &AssignmentTerms) -> Option<Self> {
        match action {
            TermsAction::Deadline => Some(Self::Deadline),
            TermsAction::Policy if terms.grading().is_scored() => Some(Self::Policy),
            TermsAction::Penalty if terms.deducted_from.is_some() => Some(Self::Penalty),
            TermsAction::Period if terms.deducted_from.is_some() => Some(Self::Period),
            TermsAction::Periods if terms.deducted_from.is_some() => Some(Self::Periods),
            TermsAction::Makeup if terms.makeup_enabled => Some(Self::Makeup),
            _ => None,
        }
    }

    fn input_mode(self) -> InputMode {
        match self {
            Self::Period => InputMode::Manual,
            _ => InputMode::Buttons,
        }
    }
}

#[derive(Clone)]
pub(super) struct TermsForm {
    pub terms: AssignmentTerms,
    pub step: TermsStep,
    pub message_id: Option<MessageId>,
    pub mode: InputMode,
    pub return_to_summary: bool,
    pub previous_closes_at: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TermsInputError {
    UnexpectedStep,
    NumberOutOfRange { max: u16 },
    InvalidDurationDate,
    DeadlineInPast,
    Domain(DomainError),
}

impl TermsForm {
    pub fn new(terms: AssignmentTerms, step: TermsStep) -> Self {
        Self {
            terms,
            step,
            message_id: None,
            mode: step.input_mode(),
            return_to_summary: false,
            previous_closes_at: None,
        }
    }

    pub fn validate_for_save(&self, now: DateTime<Utc>) -> Result<(), TermsInputError> {
        self.terms.validate().map_err(TermsInputError::Domain)?;
        match self.terms.deadline_at {
            Some(deadline) => self.terms.ensure_deadline_ahead(deadline, now),
            None => Ok(()),
        }
    }

    pub fn previous_step(&self) -> Option<TermsStep> {
        if self.return_to_summary && self.step != TermsStep::Summary {
            Some(TermsStep::Summary)
        } else {
            self.step.previous(&self.terms)
        }
    }

    pub fn go(&mut self, step: TermsStep) {
        self.step = if step == TermsStep::Summary {
            self.terms.first_incomplete_step()
        } else {
            step
        };
        if self.step == TermsStep::Summary {
            self.return_to_summary = false;
        }
        self.mode = self.step.input_mode();
    }

    pub fn advance(&mut self) {
        let step = if self.return_to_summary {
            TermsStep::Summary
        } else {
            self.step.next(&self.terms)
        };
        self.go(step);
    }

    pub fn choose_policy(
        &mut self,
        deducted_from: Option<DeductedFrom>,
    ) -> Result<(), TermsInputError> {
        if self.step != TermsStep::Policy || !self.terms.grading().is_scored() {
            return Err(TermsInputError::UnexpectedStep);
        }
        let mut terms = self.terms.clone();
        terms.deducted_from = deducted_from;
        // Сохранённый длинный горизонт можно править с итога; непредставимую дату показать нельзя.
        if terms.cannot_be_summarised() {
            return Err(TermsInputError::Domain(
                DomainError::AssignmentClosingOutOfRange,
            ));
        }
        self.terms = terms;
        self.advance();
        Ok(())
    }

    pub fn choose_makeup(&mut self, makeup: MakeupTerms) -> Result<(), TermsInputError> {
        if self.step != TermsStep::Makeup || !self.terms.makeup_enabled {
            return Err(TermsInputError::UnexpectedStep);
        }
        makeup
            .ensure_fits(self.terms.grading())
            .map_err(TermsInputError::Domain)?;
        self.terms.makeup = Some(makeup);
        self.advance();
        Ok(())
    }

    pub fn apply_number(&mut self, value: u16) -> Result<(), TermsInputError> {
        let mut terms = self.terms.clone();
        let max = match self.step {
            TermsStep::Penalty => {
                terms.penalty_per_period = Some(value);
                MAX_PENALTY_PER_PERIOD
            }
            TermsStep::Period => {
                terms.period_days = value;
                MAX_LATE_PERIOD_DAYS
            }
            TermsStep::Periods => {
                terms.max_periods = Some(value);
                MAX_LATE_PERIODS
            }
            _ => return Err(TermsInputError::UnexpectedStep),
        };
        if !(1..=max).contains(&value) {
            return Err(TermsInputError::NumberOutOfRange { max });
        }
        terms.validate().map_err(TermsInputError::Domain)?;
        self.terms = terms;
        self.advance();
        Ok(())
    }

    pub fn apply_duration_periods(&mut self, periods: u16) -> Result<(), TermsInputError> {
        if self.step != TermsStep::DurationDate {
            return Err(TermsInputError::UnexpectedStep);
        }
        let mut form = self.clone();
        form.step = TermsStep::Periods;
        form.apply_number(periods)?;
        *self = form;
        Ok(())
    }

    pub fn confirm_duration(&mut self) -> bool {
        let TermsStep::DurationConfirm { periods, .. } = self.step else {
            return false;
        };
        self.terms.max_periods = Some(periods);
        self.advance();
        true
    }

    pub fn edit_summary(&mut self, action: TermsAction) -> bool {
        if self.step != TermsStep::Summary {
            return false;
        }
        let Some(complete) = self.terms.complete() else {
            return false;
        };
        let Some(step) = TermsStep::summary_edit(action, &self.terms) else {
            return false;
        };
        self.previous_closes_at.get_or_insert(complete.closes_at);
        self.go(step);
        self.return_to_summary = true;
        true
    }
}

impl Form for TermsForm {
    type Step = TermsStep;
    type InputError = TermsInputError;

    fn step(&self) -> TermsStep {
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
        // Из итога /cancel ведёт на шаг назад. Всю форму отменяет отдельная кнопка.
        self.step != TermsStep::Summary && self.previous_step().is_some()
    }

    fn back(&mut self) -> bool {
        match (self.mode, self.step) {
            (InputMode::Manual, step) if step != TermsStep::Period => {
                self.mode = InputMode::Buttons
            }
            (InputMode::Calendar(_), _) => self.mode = InputMode::Buttons,
            _ => {
                let Some(step) = self.previous_step() else {
                    return false;
                };
                // Незавершённую политику нельзя показать в итоге, но её числа пригодятся при повторном включении.
                if step == TermsStep::Summary && self.terms.complete().is_none() {
                    self.terms.deducted_from = None;
                }
                self.go(step);
            }
        }
        true
    }

    fn accepts_text(&self) -> bool {
        match self.step {
            TermsStep::Deadline
            | TermsStep::DurationDate
            | TermsStep::Penalty
            | TermsStep::Period
            | TermsStep::Periods => true,
            TermsStep::Makeup => self.terms.grading().is_scored(),
            TermsStep::Policy | TermsStep::DurationConfirm { .. } | TermsStep::Summary => false,
        }
    }

    fn apply_date(&mut self, at: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), TermsInputError> {
        if self.step == TermsStep::DurationDate {
            let deadline = self
                .terms
                .deadline_at
                .ok_or(TermsInputError::UnexpectedStep)?;
            let policy = self
                .terms
                .preview_policy(1)
                .ok_or(TermsInputError::UnexpectedStep)?;
            let periods = u16::try_from(policy.periods_to_close_at(deadline, at))
                .ok()
                .filter(|periods| (1..=MAX_LATE_PERIODS).contains(periods))
                .ok_or(TermsInputError::InvalidDurationDate)?;
            let mut terms = self.terms.clone();
            terms.max_periods = Some(periods);
            terms.validate().map_err(|error| match error {
                DomainError::LateHorizonTooLong { .. } | DomainError::InvalidLatePolicy { .. } => {
                    TermsInputError::InvalidDurationDate
                }
                _ => TermsInputError::Domain(error),
            })?;
            self.go(TermsStep::DurationConfirm {
                selected: at,
                periods,
            });
        } else {
            if self.step != TermsStep::Deadline {
                return Err(TermsInputError::UnexpectedStep);
            }
            self.terms.ensure_deadline_ahead(at, now)?;
            let mut terms = self.terms.clone();
            terms.deadline_at = Some(at);
            terms.validate().map_err(TermsInputError::Domain)?;
            self.terms = terms;
            self.advance();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::domain::LATEST_TERM_YEAR;
    use crate::test_support::t0;

    fn last_supported_day() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(LATEST_TERM_YEAR, 12, 31, 0, 0, 0)
            .unwrap()
    }

    fn scored_terms() -> AssignmentTerms {
        let mut draft = DraftAssignment::new(
            AssignmentText {
                course_id: TeachingCourseId::new(),
                title: "Title".into(),
                body: "Body".into(),
            },
            IssuedGroups::Single(StudentGroupId::new()),
            1,
        );
        draft.grading = Grading::scored(5).unwrap();
        AssignmentTerms::creating(draft, MakeupQuestion::default())
    }

    fn complete_form() -> TermsForm {
        TermsForm::new(
            AssignmentTerms {
                deadline_at: Some(t0() + Duration::days(7)),
                deducted_from: Some(DeductedFrom::WorkScore),
                penalty_per_period: Some(2),
                max_periods: Some(4),
                ..scored_terms()
            },
            TermsStep::Summary,
        )
    }

    #[test]
    fn repeated_back_after_a_point_edit_reaches_the_exit_instead_of_looping() {
        let mut form = complete_form();
        let before = form.terms.complete();
        assert!(form.edit_summary(TermsAction::Deadline));

        for step in [
            TermsStep::Summary,
            TermsStep::Periods,
            TermsStep::Penalty,
            TermsStep::Deadline,
            TermsStep::Policy,
        ] {
            assert!(form.back());
            assert_eq!(form.step, step);
        }
        assert!(!form.back());
        assert_eq!(form.terms.complete(), before);
    }

    #[test]
    fn changing_the_period_keeps_the_penalty_and_count_and_returns_to_summary() {
        let mut form = complete_form();
        let old_closing = form.terms.complete().unwrap().closes_at;
        assert!(form.edit_summary(TermsAction::Period));
        assert_eq!(form.mode, InputMode::Manual);

        form.apply_number(10).unwrap();

        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(
            form.terms.complete().unwrap().late_policy,
            Some(LatePolicy::new(10, 2, 4, DeductedFrom::WorkScore).unwrap())
        );
        assert_eq!(form.previous_closes_at, Some(old_closing));
    }

    #[test]
    fn cancelling_an_incomplete_policy_restores_a_strict_summary_but_keeps_entered_numbers() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(t0() + Duration::days(7));
        let mut form = TermsForm::new(terms, TermsStep::Summary);
        assert!(form.edit_summary(TermsAction::Policy));
        form.choose_policy(Some(DeductedFrom::CourseTotal)).unwrap();
        form.apply_number(2).unwrap();
        assert_eq!(form.step, TermsStep::Periods);

        assert!(form.back());

        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(form.terms.deducted_from, None);
        assert_eq!(form.terms.complete().unwrap().late_policy, None);
        assert_eq!(form.terms.penalty_per_period, Some(2));
        assert_eq!(form.terms.max_periods, None);
    }

    #[test]
    fn the_policy_step_sets_the_deduction_or_closes_submissions_at_the_deadline() {
        for deducted_from in [
            None,
            Some(DeductedFrom::WorkScore),
            Some(DeductedFrom::CourseTotal),
        ] {
            let mut form = TermsForm::new(scored_terms(), TermsStep::Policy);

            form.choose_policy(deducted_from).unwrap();

            assert_eq!(form.terms.deducted_from, deducted_from);
            assert_eq!(form.step, TermsStep::Deadline);
            form.apply_date(t0() + Duration::days(7), t0()).unwrap();
            let next = if deducted_from.is_some() {
                TermsStep::Penalty
            } else {
                TermsStep::Summary
            };
            assert_eq!(form.step, next, "{deducted_from:?}");
        }
    }

    #[test]
    fn changing_the_deduction_from_the_summary_keeps_every_other_condition() {
        let mut form = complete_form();
        let before = form.terms.complete().unwrap();
        assert!(form.edit_summary(TermsAction::Policy));

        form.choose_policy(Some(DeductedFrom::CourseTotal)).unwrap();

        assert_eq!(form.step, TermsStep::Summary);
        let after = form.terms.complete().unwrap();
        assert_eq!(after.deadline_at, before.deadline_at);
        assert_eq!(after.closes_at, before.closes_at);
        assert_eq!(
            after.late_policy,
            Some(LatePolicy::new(7, 2, 4, DeductedFrom::CourseTotal).unwrap())
        );
    }

    #[test]
    fn a_duration_date_changes_the_period_count_only_after_confirmation() {
        let mut form = complete_form();
        let before = form.terms.complete();
        let deadline = form.terms.deadline_at.unwrap();
        assert!(form.edit_summary(TermsAction::Periods));
        form.go(TermsStep::DurationDate);

        let selected = deadline + Duration::days(10);
        form.apply_date(selected, t0()).unwrap();

        assert_eq!(
            form.step,
            TermsStep::DurationConfirm {
                selected,
                periods: 2
            }
        );
        assert_eq!(form.terms.complete(), before);
        assert!(form.confirm_duration());
        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(
            form.terms.complete().unwrap().closes_at,
            deadline + Duration::days(14)
        );
        assert!(!form.confirm_duration());
    }

    #[test]
    fn invalid_period_counts_preserve_the_previous_terms_and_input_step() {
        let mut form = complete_form();
        assert!(form.edit_summary(TermsAction::Periods));
        let before = form.terms.complete();
        for (value, expected) in [
            (0, TermsInputError::NumberOutOfRange { max: 100 }),
            (101, TermsInputError::NumberOutOfRange { max: 100 }),
            (
                53,
                TermsInputError::Domain(DomainError::LateHorizonTooLong { max_days: 366 }),
            ),
        ] {
            assert_eq!(form.apply_number(value), Err(expected), "{value}");
            assert_eq!(form.terms.complete(), before, "{value}");
            assert_eq!(form.step, TermsStep::Periods);
        }
        form.apply_number(3).unwrap();
        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(form.terms.max_periods, Some(3));
    }

    #[test]
    fn a_duration_preset_sets_the_period_count_without_a_confirmation() {
        let mut form = complete_form();
        form.go(TermsStep::DurationDate);

        assert_eq!(form.apply_duration_periods(3), Ok(()));

        assert_eq!(form.terms.max_periods, Some(3));
        assert_eq!(form.step, TermsStep::Summary);
    }

    #[test]
    fn a_duration_preset_is_refused_outside_the_date_step_and_beyond_the_horizon() {
        let mut form = complete_form();
        form.go(TermsStep::Periods);
        assert_eq!(
            form.apply_duration_periods(1),
            Err(TermsInputError::UnexpectedStep)
        );

        form.go(TermsStep::DurationDate);
        let before = form.terms.complete();
        assert!(form.apply_duration_periods(MAX_LATE_PERIODS + 1).is_err());
        assert_eq!(form.step, TermsStep::DurationDate);
        assert_eq!(form.terms.complete(), before);
    }

    #[test]
    fn invalid_duration_dates_preserve_the_calendar_and_report_a_date_error() {
        let mut form = complete_form();
        let before = form.terms.complete();
        let deadline = form.terms.deadline_at.unwrap();
        form.go(TermsStep::DurationDate);
        let calendar = InputMode::Calendar(deadline.date_naive());
        form.mode = calendar;
        for at in [
            deadline - Duration::days(1),
            deadline,
            deadline + Duration::weeks(53),
            deadline + Duration::weeks(101),
        ] {
            assert_eq!(
                form.apply_date(at, t0()),
                Err(TermsInputError::InvalidDurationDate),
                "{at}"
            );
            assert_eq!(form.step, TermsStep::DurationDate);
            assert_eq!(form.mode, calendar);
            assert_eq!(form.terms.complete(), before);
        }
    }

    #[test]
    fn back_from_manual_input_returns_to_the_same_step_without_changing_terms() {
        let mut form = complete_form();
        form.go(TermsStep::Periods);
        form.mode = InputMode::Manual;
        let before = form.terms.complete();

        assert!(form.back());

        assert_eq!(form.step, TermsStep::Periods);
        assert_eq!(form.mode, InputMode::Buttons);
        assert_eq!(form.terms.complete(), before);
    }

    #[test]
    fn the_back_button_leaves_the_calendar_for_the_previous_step_unlike_cancel() {
        let mut form = complete_form();
        form.go(TermsStep::Deadline);
        let calendar = InputMode::Calendar(t0().date_naive());
        form.mode = calendar;
        let mut cancelled = form.clone();

        assert!(form.step_back());
        assert_eq!(form.step, TermsStep::Policy);
        assert!(cancelled.back());
        assert_eq!(cancelled.step, TermsStep::Deadline);
        assert_eq!(cancelled.mode, InputMode::Buttons);
    }

    #[test]
    fn only_creation_requires_a_deadline_in_the_future() {
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let mut creation = complete_form();
        creation.go(TermsStep::Deadline);
        let before = creation.terms.complete();
        let now = t0() + Duration::days(3);
        let past = t0() + Duration::days(1);
        for at in [past, now] {
            assert_eq!(
                creation.apply_date(at, now),
                Err(TermsInputError::DeadlineInPast)
            );
            assert_eq!(creation.terms.complete(), before);
        }

        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let assignment = assignment(&course, &teacher).build();
        let mut edit = TermsForm::new(
            AssignmentTerms::editing(&assignment, 0, MakeupQuestion::default()),
            TermsStep::Deadline,
        );
        edit.apply_date(past, now).unwrap();
        assert_eq!(edit.terms.deadline_at, Some(past));
        assert_eq!(edit.step, TermsStep::Summary);
    }

    #[test]
    fn saving_a_new_draft_rechecks_that_the_deadline_is_still_ahead() {
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let creation = complete_form();
        let deadline = creation.terms.deadline_at.unwrap();
        assert_eq!(
            creation.validate_for_save(deadline - Duration::seconds(1)),
            Ok(())
        );
        assert_eq!(
            creation.validate_for_save(deadline),
            Err(TermsInputError::DeadlineInPast)
        );

        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let assignment = assignment(&course, &teacher).build();
        let edit = TermsForm::new(
            AssignmentTerms::editing(&assignment, 0, MakeupQuestion::default()),
            TermsStep::Summary,
        );
        let deadline = edit.terms.deadline_at.unwrap();
        assert_eq!(edit.validate_for_save(deadline + Duration::days(1)), Ok(()));
    }

    #[test]
    fn preview_defaults_never_complete_the_policy_for_saving() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(t0());
        assert_eq!(terms.preview_policy(4), None);
        terms.deducted_from = Some(DeductedFrom::CourseTotal);

        assert_eq!(
            terms.preview_policy(4).unwrap(),
            LatePolicy::new(7, 1, 4, DeductedFrom::CourseTotal).unwrap()
        );
        assert_eq!(terms.complete(), None);
        assert_eq!(terms.penalty_per_period, None);
        assert_eq!(terms.max_periods, None);

        terms.penalty_per_period = Some(2);
        assert_eq!(terms.complete(), None);
        terms.max_periods = Some(3);
        assert_eq!(
            terms.complete().unwrap().late_policy,
            Some(terms.preview_policy(3).unwrap())
        );
        terms.deducted_from = None;
        assert_eq!(terms.complete().unwrap().late_policy, None);
    }

    #[test]
    fn complete_terms_close_at_the_deadline_or_after_the_last_late_period() {
        let mut terms = scored_terms();
        assert_eq!(terms.complete(), None);

        terms.deadline_at = Some(t0());
        assert_eq!(terms.complete().unwrap().closes_at, t0());

        terms.deducted_from = Some(DeductedFrom::WorkScore);
        terms.penalty_per_period = Some(1);
        terms.max_periods = Some(4);
        assert_eq!(
            terms.complete().unwrap().closes_at,
            t0() + Duration::days(28)
        );
    }

    #[test]
    fn a_deadline_after_the_last_supported_year_is_rejected_at_input() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(last_supported_day() + Duration::days(1));

        assert_eq!(
            terms.validate(),
            Err(DomainError::AssignmentClosingOutOfRange)
        );
    }

    #[test]
    fn a_policy_restored_over_a_deadline_it_cannot_follow_cannot_be_summarised() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(last_supported_day());
        terms.penalty_per_period = Some(1);
        terms.max_periods = Some(4);
        assert!(terms.validate().is_ok());
        assert!(!terms.cannot_be_summarised());

        terms.deducted_from = Some(DeductedFrom::WorkScore);

        assert_eq!(
            terms.validate(),
            Err(DomainError::AssignmentClosingOutOfRange)
        );
        assert!(terms.cannot_be_summarised());
    }

    #[test]
    fn a_stored_horizon_beyond_the_input_limit_can_still_be_summarised() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(t0());
        terms.deducted_from = Some(DeductedFrom::WorkScore);
        terms.period_days = 365;
        terms.penalty_per_period = Some(1);
        terms.max_periods = Some(100);

        assert!(matches!(
            terms.validate(),
            Err(DomainError::LateHorizonTooLong { .. })
        ));
        assert!(!terms.cannot_be_summarised());
    }

    #[test]
    fn editing_an_assignment_starts_from_its_current_terms() {
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let course = course_for(&student_group("test-group"), &teacher);
        let strict = assignment(&course, &teacher).build();
        let late = assignment(&course, &teacher)
            .scored(10)
            .late_policy(3, 2, 5, DeductedFrom::CourseTotal)
            .build();

        let terms = AssignmentTerms::editing(&strict, 4, MakeupQuestion::default());
        assert_eq!(terms.deducted_from, None);
        assert_eq!(terms.complete().unwrap().closes_at, strict.closes_at());

        let terms = AssignmentTerms::editing(&late, 4, MakeupQuestion::default());
        assert_eq!(terms.deducted_from, Some(DeductedFrom::CourseTotal));
        assert_eq!(terms.complete().unwrap().late_policy, late.late_policy());
        assert_eq!(terms.complete().unwrap().closes_at, late.closes_at());
        assert!(
            matches!(terms.target, TermsTarget::Edit { revision: 4, assignment_id, .. }
                if assignment_id == late.id())
        );
    }

    #[test]
    fn summary_transition_collects_missing_policy_fields_without_defaulting_them() {
        let mut terms = scored_terms();
        terms.deducted_from = Some(DeductedFrom::WorkScore);
        let mut form = TermsForm::new(terms, TermsStep::Policy);

        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Deadline);
        form.terms.deadline_at = Some(t0());
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Penalty);
        assert!(form.terms.penalty_per_period.is_none());
        form.terms.penalty_per_period = Some(2);
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Periods);
        assert!(form.terms.max_periods.is_none());
        form.terms.max_periods = Some(4);
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(
            form.terms.complete().unwrap().late_policy,
            Some(LatePolicy::new(7, 2, 4, DeductedFrom::WorkScore).unwrap())
        );
    }

    fn makeup_terms(enabled: bool) -> AssignmentTerms {
        let mut draft = DraftAssignment::new(
            AssignmentText {
                course_id: TeachingCourseId::new(),
                title: "Title".into(),
                body: "Body".into(),
            },
            IssuedGroups::Single(StudentGroupId::new()),
            1,
        );
        draft.grading = Grading::scored(10).unwrap();
        AssignmentTerms::creating(
            draft,
            MakeupQuestion {
                enabled,
                hint: None,
            },
        )
    }

    #[test]
    fn the_makeup_question_follows_the_scale_only_on_a_course_with_makeup() {
        let enabled = makeup_terms(true);
        assert_eq!(enabled.first_step(), TermsStep::Makeup);
        let mut form = TermsForm::new(enabled, TermsStep::Makeup);
        assert!(!form.can_go_back());
        form.choose_makeup(MakeupTerms::scored(6, form.terms.grading()).unwrap())
            .unwrap();
        assert_eq!(form.step, TermsStep::Policy);
        assert!(form.back());
        assert_eq!(form.step, TermsStep::Makeup);
        assert_eq!(
            form.terms.makeup,
            Some(MakeupTerms::scored(6, form.terms.grading()).unwrap())
        );

        let disabled = makeup_terms(false);
        assert_eq!(disabled.first_step(), TermsStep::Policy);
        let mut form = TermsForm::new(disabled, TermsStep::Policy);
        assert_eq!(
            form.choose_makeup(MakeupTerms::NotOffered),
            Err(TermsInputError::UnexpectedStep)
        );
        assert!(!form.back());
    }

    #[test]
    fn a_pass_fail_assignment_goes_from_the_makeup_question_to_the_strict_deadline() {
        let mut terms = makeup_terms(true);
        if let TermsTarget::Create(draft) = &mut terms.target {
            draft.grading = Grading::PassFail;
        }
        let mut form = TermsForm::new(terms, TermsStep::Makeup);

        assert_eq!(
            form.choose_makeup(MakeupTerms::scored(6, Grading::scored(10).unwrap()).unwrap()),
            Err(TermsInputError::Domain(
                DomainError::MakeupTermsDoNotFitGrading
            ))
        );
        form.choose_makeup(MakeupTerms::PassFail).unwrap();
        assert_eq!(form.step, TermsStep::Deadline);
        assert!(form.back());
        assert_eq!(form.step, TermsStep::Makeup);
    }

    #[test]
    fn changing_the_makeup_terms_from_the_summary_returns_to_the_summary() {
        let mut form = TermsForm::new(
            AssignmentTerms {
                deadline_at: Some(t0() + Duration::days(7)),
                makeup: Some(MakeupTerms::scored(6, Grading::scored(10).unwrap()).unwrap()),
                ..makeup_terms(true)
            },
            TermsStep::Summary,
        );

        assert!(form.edit_summary(TermsAction::Makeup));
        assert_eq!(form.step, TermsStep::Makeup);
        form.choose_makeup(MakeupTerms::NotOffered).unwrap();

        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(form.terms.makeup, Some(MakeupTerms::NotOffered));
        let mut without = TermsForm::new(
            AssignmentTerms {
                deadline_at: Some(t0() + Duration::days(7)),
                ..makeup_terms(false)
            },
            TermsStep::Summary,
        );
        assert!(!without.edit_summary(TermsAction::Makeup));
    }
}
