use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use crate::domain::{
    AssignmentId, Grading, InviteGeneration, LatePolicy, ReviewDecision, StoredFileId,
    SubmissionId, TeachingCourseId, UserId,
};
use crate::invites::VerifiedInvite;

pub(super) type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

//Шагов создания больше, чем полей у варианта состояния,
// поэтому черновик едет одной структурой
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
    ChoosingGradebookStudent {
        message_id: MessageId,
        course_id: TeachingCourseId,
        student_ids: Vec<UserId>,
    },
    ChoosingAttemptStudent {
        message_id: MessageId,
        assignment_id: AssignmentId,
        student_ids: Vec<UserId>,
    },
    AwaitingAttemptAmount {
        message_id: MessageId,
        assignment_id: AssignmentId,
        student_id: UserId,
    },
}
