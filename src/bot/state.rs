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

    pub(super) fn choosing_late_policy(self, message_id: MessageId) -> Option<DraftAssignment> {
        match self {
            Self::Flow(FlowState::ChoosingLatePolicy {
                message_id: current,
                draft,
            }) if current == message_id => Some(draft),
            _ => None,
        }
    }

    pub(super) fn choosing_deadline(self, message_id: MessageId) -> Option<DeadlineTarget> {
        match self {
            Self::Flow(FlowState::ChoosingDeadline {
                message_id: current,
                target,
            })
            | Self::Flow(FlowState::PickingDeadlineDate {
                message_id: current,
                target,
            }) if current == message_id => Some(target),
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
    pub late_policy: Option<LatePolicy>,
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
            late_policy: None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum StudentChoice {
    Grades { course_id: TeachingCourseId },
    ExtraAttempts { assignment_id: AssignmentId },
}

#[derive(Clone)]
pub(super) enum DeadlineTarget {
    Create(DraftAssignment),
    Change { assignment_id: AssignmentId },
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
    ChoosingLatePolicy {
        message_id: MessageId,
        draft: DraftAssignment,
    },
    AwaitingLatePolicyText {
        draft: DraftAssignment,
    },
    ChoosingDeadline {
        message_id: MessageId,
        target: DeadlineTarget,
    },
    PickingDeadlineDate {
        message_id: MessageId,
        target: DeadlineTarget,
    },
    AwaitingDeadlineText {
        target: DeadlineTarget,
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
