use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use crate::domain::{
    AssignmentId, ReviewDecision, StoredFileId, SubmissionId, TeachingCourseId, UserId,
};

pub(super) type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

#[derive(Clone)]
pub(super) enum DeadlineTarget {
    Create {
        course_id: TeachingCourseId,
        title: String,
        body: String,
        max_attempts: u16,
    },
    Change {
        assignment_id: AssignmentId,
    },
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
}

#[derive(Clone)]
pub(super) enum FlowState {
    AwaitingRegistrationName,
    AwaitingRegistrationGroup {
        display_name: String,
        group_codes: Vec<String>,
    },
    AwaitingSubmission {
        assignment_id: AssignmentId,
    },
    AwaitingReviewDecision {
        submission_id: SubmissionId,
    },
    AwaitingReviewComment {
        submission_id: SubmissionId,
        decision: ReviewDecision,
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
