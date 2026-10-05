use chrono::{DateTime, Utc};
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use super::assignment_form::{AssignmentText, DraftAssignment, InputMode, TermsForm, TermsStep};
use super::views::MenuAction;
use crate::domain::{
    AssignmentId, Grading, InviteGeneration, ReviewDecision, StoredFileId, SubmissionId,
    TeachingCourseId, TelegramUserId, UserId,
};
use crate::invites::VerifiedInvite;

pub(super) type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

pub(super) struct FilesScreen<'a> {
    pub assignment_id: AssignmentId,
    pub revision: i64,
    pub creation_flow: bool,
    pub requires_confirmation: bool,
    pub file_ids: &'a [StoredFileId],
}

#[derive(Clone, Copy)]
pub(super) struct FileDeletion {
    pub assignment_id: AssignmentId,
    pub file_id: StoredFileId,
    pub expected_revision: i64,
    pub creation_flow: bool,
}

/// Кнопка допустима при совпадении двух условий: ожидаемого состояния диалога и ID сообщения.
/// Оба условия проверяются в каждом методе разбора.
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

    pub(super) fn sandbox_screen(&self, message_id: MessageId) -> Option<DateTime<Utc>> {
        match self {
            Self::Screen(ScreenState::SandboxSubmissions {
                message_id: current,
                shown_at,
            }) if *current == message_id => Some(*shown_at),
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

    pub(super) fn files_screen(&self, message_id: MessageId) -> Option<FilesScreen<'_>> {
        match self {
            Self::Screen(ScreenState::ManagingAssignmentFiles {
                message_id: current,
                assignment_id,
                revision,
                creation_flow,
                deletion_requires_confirmation,
                file_ids,
            }) if *current == message_id => Some(FilesScreen {
                assignment_id: *assignment_id,
                revision: *revision,
                creation_flow: *creation_flow,
                requires_confirmation: *deletion_requires_confirmation,
                file_ids,
            }),
            _ => None,
        }
    }

    pub(super) fn confirming_file_deletion(&self, message_id: MessageId) -> Option<FileDeletion> {
        match self {
            Self::Flow(FlowState::ConfirmingAssignmentFileDeletion {
                message_id: current,
                deletion,
            }) if *current == message_id => Some(*deletion),
            _ => None,
        }
    }

    /// ID студента принимается только из показанного списка, чтобы отсечь чужие кнопки.
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

    pub(super) fn awaiting_max_attempts(self, message_id: MessageId) -> Option<AssignmentText> {
        match self {
            Self::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: current,
                text,
            }) if current == message_id => Some(text),
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

#[derive(Clone, Copy)]
pub(super) enum StudentChoice {
    Grades { course_id: TeachingCourseId },
    ExtraAttempts { assignment_id: AssignmentId },
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
    SandboxSubmissions {
        message_id: MessageId,
        shown_at: DateTime<Utc>,
    },
}

#[derive(Clone)]
pub(super) enum FlowState {
    /// После проверки подписи исходный bearer credential больше не хранится.
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
    AwaitingBulkReviewComment {
        decision: ReviewDecision,
        cutoff: DateTime<Utc>,
    },
    AwaitingAssignmentFile {
        assignment_id: AssignmentId,
        expected_revision: i64,
        creation_flow: bool,
    },
    ConfirmingAssignmentFileDeletion {
        message_id: MessageId,
        deletion: FileDeletion,
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
        text: AssignmentText,
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
        action: MenuAction,
        telegram_user_id: TelegramUserId,
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
    fn files_screen_requires_current_keyboard_and_preserves_file_order() {
        let assignment_id = AssignmentId::new();
        let file_ids = vec![StoredFileId::new(), StoredFileId::new()];
        let state = BotState::Screen(ScreenState::ManagingAssignmentFiles {
            message_id: MessageId(10),
            assignment_id,
            revision: 7,
            creation_flow: true,
            deletion_requires_confirmation: true,
            file_ids: file_ids.clone(),
        });

        let files = state.files_screen(MessageId(10)).unwrap();
        assert_eq!(files.assignment_id, assignment_id);
        assert_eq!(files.revision, 7);
        assert!(files.creation_flow);
        assert!(files.requires_confirmation);
        assert_eq!(files.file_ids, file_ids);
        assert!(files.file_ids.get(2).is_none());
        assert!(state.files_screen(MessageId(11)).is_none());
        assert!(BotState::Idle.files_screen(MessageId(10)).is_none());
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

    #[test]
    fn bulk_review_requires_the_current_sandbox_screen() {
        let shown_at = crate::test_support::t0();
        let state = BotState::Screen(ScreenState::SandboxSubmissions {
            message_id: MessageId(10),
            shown_at,
        });

        assert_eq!(state.sandbox_screen(MessageId(10)), Some(shown_at));
        assert!(state.sandbox_screen(MessageId(11)).is_none());
        assert!(BotState::Idle.sandbox_screen(MessageId(10)).is_none());
    }
}
