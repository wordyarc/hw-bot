use chrono::{DateTime, Utc};
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use crate::domain::{
    AssignmentId, Grading, InviteGeneration, LatePolicy, ReviewDecision, StoredFileId,
    SubmissionId, TeachingCourseId, UserId,
};
use crate::invites::VerifiedInvite;

pub(super) type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

/// Разбор состояния под нажатую кнопку. У каждого метода один и тот же инвариант: диалог обязан
/// быть в ожидаемом состоянии, а нажатие - прийти из того самого сообщения.
impl BotState {
    pub(super) fn managing_assignment(
        &self,
        message_id: MessageId,
        assignment_id: AssignmentId,
    ) -> Option<AssignmentId> {
        match self {
            Self::Screen(ScreenState::ManagingAssignment {
                message_id: current_message,
                assignment_id: current_assignment,
            }) if *current_message == message_id && *current_assignment == assignment_id => {
                Some(assignment_id)
            }
            _ => None,
        }
    }

    pub(super) fn group_invite(
        &self,
        message_id: MessageId,
    ) -> Option<(TeachingCourseId, InviteGeneration)> {
        match self {
            Self::Screen(ScreenState::GroupInvite {
                message_id: current,
                course_id,
                generation,
            }) if *current == message_id => Some((*course_id, *generation)),
            _ => None,
        }
    }

    pub(super) fn confirming_invite_rotation(
        &self,
        message_id: MessageId,
    ) -> Option<(TeachingCourseId, InviteGeneration)> {
        match self {
            Self::Flow(FlowState::ConfirmingInviteRotation {
                message_id: current,
                course_id,
                generation,
            }) if *current == message_id => Some((*course_id, *generation)),
            _ => None,
        }
    }

    pub(super) fn managing_assignment_files(
        &self,
        message_id: MessageId,
    ) -> Option<(AssignmentId, i64, bool)> {
        match self {
            Self::Screen(ScreenState::ManagingAssignmentFiles {
                message_id: current,
                assignment_id,
                revision,
                creation_flow,
                ..
            }) if *current == message_id => Some((*assignment_id, *revision, *creation_flow)),
            _ => None,
        }
    }

    pub(super) fn confirming_file_deletion(
        &self,
        message_id: MessageId,
    ) -> Option<(AssignmentId, StoredFileId, i64, bool)> {
        match self {
            Self::Flow(FlowState::ConfirmingAssignmentFileDeletion {
                message_id: current,
                assignment_id,
                file_id,
                expected_revision,
                creation_flow,
            }) if *current == message_id => {
                Some((*assignment_id, *file_id, *expected_revision, *creation_flow))
            }
            _ => None,
        }
    }

    /// Студент обязан быть из того же списка, который бот показал: иначе кнопка чужая.
    pub(super) fn choosing_student(
        &self,
        message_id: MessageId,
        student_id: UserId,
    ) -> Option<StudentChoice> {
        match self {
            Self::Flow(FlowState::ChoosingStudent {
                message_id: current,
                purpose,
                student_ids,
            }) if *current == message_id && student_ids.contains(&student_id) => Some(*purpose),
            _ => None,
        }
    }

    pub(super) fn awaiting_attempt_amount(
        &self,
        message_id: MessageId,
    ) -> Option<(AssignmentId, UserId)> {
        match self {
            Self::Flow(FlowState::AwaitingAttemptAmount {
                message_id: current,
                assignment_id,
                student_id,
            }) if *current == message_id => Some((*assignment_id, *student_id)),
            _ => None,
        }
    }

    pub(super) fn awaiting_max_attempts(
        self,
        message_id: MessageId,
    ) -> Option<(TeachingCourseId, String, String)> {
        match self {
            Self::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: current,
                course_id,
                title,
                body,
            }) if current == message_id => Some((course_id, title, body)),
            _ => None,
        }
    }

    pub(super) fn choosing_grading(self, message_id: MessageId) -> Option<DraftAssignment> {
        match self {
            Self::Flow(FlowState::ChoosingGrading {
                message_id: current,
                draft,
            }) if current == message_id => Some(draft),
            _ => None,
        }
    }

    pub(super) fn terms_form(self, message_id: MessageId, step: TermsStep) -> Option<TermsForm> {
        match self {
            Self::Flow(FlowState::ConfiguringTerms(form))
                if form.message_id == Some(message_id)
                    && form.step == step
                    && form.mode != InputMode::Manual =>
            {
                Some(form)
            }
            _ => None,
        }
    }

    pub(super) fn active_terms(&self) -> Option<&TermsForm> {
        match self {
            Self::Flow(FlowState::ConfiguringTerms(form)) => Some(form),
            Self::Flow(FlowState::ConfirmingTermsExit { form, .. }) => Some(form),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct DraftAssignment {
    pub course_id: TeachingCourseId,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub grading: Grading,
}

impl DraftAssignment {
    pub fn new(
        course_id: TeachingCourseId,
        title: String,
        body: String,
        max_attempts: u16,
    ) -> Self {
        Self {
            course_id,
            title,
            body,
            max_attempts,
            grading: Grading::PassFail,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum StudentChoice {
    Grades { course_id: TeachingCourseId },
    ExtraAttempts { assignment_id: AssignmentId },
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

impl AssignmentTerms {
    pub fn first_incomplete_step(&self) -> TermsStep {
        let mut step = TermsStep::Deadline;
        while step.is_complete(self) {
            step = step.next(self);
        }
        step
    }

    pub fn creating(draft: DraftAssignment) -> Self {
        Self {
            target: TermsTarget::Create(draft),
            deadline_at: None,
            allow_late: false,
            period_days: 7,
            penalty: None,
            max_periods: None,
        }
    }

    pub fn committed_policy(&self) -> crate::domain::DomainResult<Option<LatePolicy>> {
        if !self.allow_late {
            return Ok(None);
        }
        self.policy(self.penalty.unwrap_or(0), self.max_periods.unwrap_or(0))
            .map(Some)
    }

    pub fn preview_policy(&self, max_periods: u16) -> crate::domain::DomainResult<LatePolicy> {
        // Превью нужно до ввода штрафа; при сохранении этот дефолт не используется.
        const PREVIEW_PENALTY: u16 = 1;
        self.policy(self.penalty.unwrap_or(PREVIEW_PENALTY), max_periods)
    }

    fn policy(&self, penalty: u16, max_periods: u16) -> crate::domain::DomainResult<LatePolicy> {
        LatePolicy::new(self.period_days, penalty, max_periods)
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

    pub fn summary_edit(
        action: crate::bot::callbacks::TermsAction,
        terms: &AssignmentTerms,
    ) -> Option<Self> {
        use crate::bot::callbacks::TermsAction;
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
    Calendar(chrono::NaiveDate),
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
    pub fn can_go_back(&self) -> bool {
        // На итоге есть отдельная отмена формы, а /cancel по-прежнему возвращает шаг.
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
}

#[derive(Clone, Default)]
pub(super) enum BotState {
    #[default]
    Idle,
    Screen(ScreenState),
    Flow(FlowState),
}

#[derive(Clone)]
pub(super) enum ScreenState {
    Guest,
    ManagingAssignment {
        message_id: MessageId,
        assignment_id: AssignmentId,
    },
    ManagingAssignmentFiles {
        message_id: MessageId,
        assignment_id: AssignmentId,
        revision: i64,
        file_ids: Vec<StoredFileId>,
        deletion_requires_confirmation: bool,
        creation_flow: bool,
    },
    GroupInvite {
        message_id: MessageId,
        course_id: TeachingCourseId,
        generation: InviteGeneration,
    },
}

#[derive(Clone)]
pub(super) enum FlowState {
    /// bearer credential не должен пережить проверку подписи.
    AwaitingRegistrationName {
        invite: VerifiedInvite,
    },
    ConfirmingInviteRotation {
        message_id: MessageId,
        course_id: TeachingCourseId,
        generation: InviteGeneration,
    },
    AwaitingSubmission {
        assignment_id: AssignmentId,
    },
    AwaitingReviewDecision {
        submission_id: SubmissionId,
        grading: Grading,
        penalty: u16,
    },
    AwaitingReviewScore {
        submission_id: SubmissionId,
        max_score: u16,
        penalty: u16,
    },
    AwaitingReviewComment {
        submission_id: SubmissionId,
        decision: ReviewDecision,
        score: Option<u16>,
    },
    AwaitingAssignmentFile {
        assignment_id: AssignmentId,
        expected_revision: i64,
        creation_flow: bool,
    },
    ConfirmingAssignmentFileDeletion {
        message_id: MessageId,
        assignment_id: AssignmentId,
        file_id: StoredFileId,
        expected_revision: i64,
        creation_flow: bool,
    },
    AwaitingAssignmentTitle {
        course_id: TeachingCourseId,
    },
    AwaitingAssignmentBody {
        course_id: TeachingCourseId,
        title: String,
    },
    AwaitingAssignmentMaxAttempts {
        message_id: MessageId,
        course_id: TeachingCourseId,
        title: String,
        body: String,
    },
    ChoosingGrading {
        message_id: MessageId,
        draft: DraftAssignment,
    },
    AwaitingGradingText {
        draft: DraftAssignment,
    },
    ConfiguringTerms(TermsForm),
    ConfirmingTermsExit {
        message_id: MessageId,
        form: TermsForm,
        action: crate::bot::views::MenuAction,
        telegram_user_id: crate::domain::TelegramUserId,
    },
    ChoosingStudent {
        message_id: MessageId,
        purpose: StudentChoice,
        student_ids: Vec<UserId>,
    },
    AwaitingAttemptAmount {
        message_id: MessageId,
        assignment_id: AssignmentId,
        student_id: UserId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_defaults_never_complete_the_policy_for_saving() {
        let draft = DraftAssignment::new(TeachingCourseId::new(), "Title".into(), "Body".into(), 1);
        let mut terms = AssignmentTerms::creating(draft);
        terms.allow_late = true;

        assert_eq!(
            terms.preview_policy(4).unwrap(),
            LatePolicy::new(7, 1, 4).unwrap()
        );
        assert!(terms.committed_policy().is_err());
        assert_eq!(terms.penalty, None);
        assert_eq!(terms.max_periods, None);

        terms.penalty = Some(2);
        assert!(terms.committed_policy().is_err());
        terms.max_periods = Some(3);
        assert_eq!(
            terms.committed_policy().unwrap(),
            Some(terms.preview_policy(3).unwrap())
        );
        terms.allow_late = false;
        assert_eq!(terms.committed_policy().unwrap(), None);
    }

    #[test]
    fn assignment_action_requires_current_keyboard_and_assignment() {
        let current = AssignmentId::new();
        let other = AssignmentId::new();
        let state = BotState::Screen(ScreenState::ManagingAssignment {
            message_id: MessageId(10),
            assignment_id: current,
        });

        assert_eq!(
            state.managing_assignment(MessageId(10), current),
            Some(current)
        );
        assert!(state.managing_assignment(MessageId(11), current).is_none());
        assert!(state.managing_assignment(MessageId(10), other).is_none());
    }
}
