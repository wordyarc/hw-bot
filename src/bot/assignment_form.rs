use chrono::{DateTime, NaiveDate, Utc};
use teloxide::types::MessageId;

use super::callbacks::TermsAction;
use crate::domain::{
    Assignment, AssignmentId, DomainError, DomainResult, Grading, LatePolicy, TeachingCourseId,
};

const DEFAULT_PERIOD_DAYS: u16 = 7;

#[derive(Clone)]
pub(super) struct AssignmentText {
    pub course_id: TeachingCourseId,
    pub title: String,
    pub body: String,
}

#[derive(Clone)]
pub(super) struct DraftAssignment {
    pub text: AssignmentText,
    pub max_attempts: u16,
    pub grading: Grading,
}

impl DraftAssignment {
    pub fn new(text: AssignmentText, max_attempts: u16) -> Self {
        Self {
            text,
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
    pub allow_late: bool,
    pub period_days: u16,
    pub penalty: Option<u16>,
    pub max_periods: Option<u16>,
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
    pub fn creating(draft: DraftAssignment) -> Self {
        Self {
            target: TermsTarget::Create(draft),
            deadline_at: None,
            allow_late: false,
            period_days: DEFAULT_PERIOD_DAYS,
            penalty: None,
            max_periods: None,
        }
    }

    pub fn editing(assignment: &Assignment, revision: i64) -> Self {
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
            allow_late: policy.is_some(),
            period_days: policy.map_or(DEFAULT_PERIOD_DAYS, LatePolicy::period_days),
            penalty: policy.map(LatePolicy::penalty),
            max_periods: policy.map(LatePolicy::max_periods),
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
        let late_policy = if self.allow_late {
            Some(LatePolicy::new(self.period_days, self.penalty?, self.max_periods?).ok()?)
        } else {
            None
        };
        let closes_at = match late_policy {
            Some(policy) => policy.closes_at(deadline_at).ok()?,
            None => deadline_at,
        };

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

    pub fn preview_policy(&self, max_periods: u16) -> DomainResult<LatePolicy> {
        // Пока штраф не введён, это значение нужно лишь для превью и в сохранённые условия
        // не попадёт.
        const PREVIEW_PENALTY: u16 = 1;
        LatePolicy::new(
            self.period_days,
            self.penalty.unwrap_or(PREVIEW_PENALTY),
            max_periods,
        )
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
        if self.allow_late {
            self.preview_policy(self.max_periods.unwrap_or(1))?
                .validate_horizon(deadline)?;
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
            Self::Policy => terms.first_incomplete_step(),
            Self::Deadline if terms.allow_late => Self::Penalty,
            Self::Deadline => Self::Summary,
            Self::Period => Self::Penalty,
            Self::Penalty => Self::Periods,
            Self::Periods | Self::DurationConfirm { .. } | Self::Summary => Self::Summary,
            Self::DurationDate => Self::DurationDate,
        }
    }

    pub fn previous(self, terms: &AssignmentTerms) -> Option<Self> {
        match self {
            Self::Policy => None,
            Self::Deadline => terms.grading().is_scored().then_some(Self::Policy),
            Self::Penalty => Some(Self::Deadline),
            Self::Period | Self::Periods => Some(Self::Penalty),
            Self::DurationDate => Some(Self::Periods),
            Self::DurationConfirm { .. } => Some(Self::DurationDate),
            Self::Summary if matches!(terms.target, TermsTarget::Edit { .. }) => None,
            Self::Summary if terms.allow_late => Some(Self::Periods),
            Self::Summary => Some(Self::Deadline),
        }
    }

    fn is_complete(self, terms: &AssignmentTerms) -> bool {
        match self {
            Self::Deadline => terms.deadline_at.is_some(),
            Self::Penalty => terms.penalty.is_some(),
            Self::Periods => terms.max_periods.is_some(),
            Self::Policy
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
            TermsAction::Penalty if terms.allow_late => Some(Self::Penalty),
            TermsAction::Period if terms.allow_late => Some(Self::Period),
            TermsAction::Periods if terms.allow_late => Some(Self::Periods),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputMode {
    Buttons,
    Manual,
    Calendar(NaiveDate),
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

    pub fn can_go_back(&self) -> bool {
        // Из итога /cancel ведёт на шаг назад. Всю форму отменяет отдельная кнопка.
        self.step != TermsStep::Summary && self.previous_step().is_some()
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
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::test_support::t0;

    fn scored_terms() -> AssignmentTerms {
        let mut draft = DraftAssignment::new(
            AssignmentText {
                course_id: TeachingCourseId::new(),
                title: "Title".into(),
                body: "Body".into(),
            },
            1,
        );
        draft.grading = Grading::scored(5).unwrap();
        AssignmentTerms::creating(draft)
    }

    #[test]
    fn preview_defaults_never_complete_the_policy_for_saving() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(t0());
        terms.allow_late = true;

        assert_eq!(
            terms.preview_policy(4).unwrap(),
            LatePolicy::new(7, 1, 4).unwrap()
        );
        assert_eq!(terms.complete(), None);
        assert_eq!(terms.penalty, None);
        assert_eq!(terms.max_periods, None);

        terms.penalty = Some(2);
        assert_eq!(terms.complete(), None);
        terms.max_periods = Some(3);
        assert_eq!(
            terms.complete().unwrap().late_policy,
            Some(terms.preview_policy(3).unwrap())
        );
        terms.allow_late = false;
        assert_eq!(terms.complete().unwrap().late_policy, None);
    }

    #[test]
    fn complete_terms_close_at_the_deadline_or_after_the_last_late_period() {
        let mut terms = scored_terms();
        assert_eq!(terms.complete(), None);

        terms.deadline_at = Some(t0());
        assert_eq!(terms.complete().unwrap().closes_at, t0());

        terms.allow_late = true;
        terms.penalty = Some(1);
        terms.max_periods = Some(4);
        assert_eq!(
            terms.complete().unwrap().closes_at,
            t0() + Duration::days(28)
        );
    }

    #[test]
    fn a_policy_restored_over_a_deadline_it_cannot_follow_cannot_be_summarised() {
        let mut terms = scored_terms();
        terms.deadline_at = Some(DateTime::<Utc>::MAX_UTC - Duration::days(1));
        terms.penalty = Some(1);
        terms.max_periods = Some(4);
        assert!(terms.validate().is_ok());
        assert!(!terms.cannot_be_summarised());

        terms.allow_late = true;

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
        terms.allow_late = true;
        terms.period_days = 365;
        terms.penalty = Some(1);
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
            .late_policy(3, 2, 5)
            .build();

        let terms = AssignmentTerms::editing(&strict, 4);
        assert!(!terms.allow_late);
        assert_eq!(terms.complete().unwrap().closes_at, strict.closes_at());

        let terms = AssignmentTerms::editing(&late, 4);
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
        terms.allow_late = true;
        let mut form = TermsForm::new(terms, TermsStep::Policy);

        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Deadline);
        form.terms.deadline_at = Some(t0());
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Penalty);
        assert!(form.terms.penalty.is_none());
        form.terms.penalty = Some(2);
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Periods);
        assert!(form.terms.max_periods.is_none());
        form.terms.max_periods = Some(4);
        form.go(TermsStep::Summary);
        assert_eq!(form.step, TermsStep::Summary);
        assert_eq!(
            form.terms.complete().unwrap().late_policy,
            Some(LatePolicy::new(7, 2, 4).unwrap())
        );
    }
}
