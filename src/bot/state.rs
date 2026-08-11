use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};

use crate::domain::{AssignmentId, ReviewDecision, SubmissionId};

pub(super) type BotDialogue = Dialogue<BotState, InMemStorage<BotState>>;

#[derive(Clone, Default)]
pub(super) enum BotState {
    #[default]
    Idle,
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
}
