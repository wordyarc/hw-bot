use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use crate::domain::{AssignmentId, ReviewDecision, SubmissionId, TeachingCourseId, UserId};

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
    ChoosingCourse {
        message_id: MessageId,
        course_ids: Vec<TeachingCourseId>,
    },
    ChoosingAssignment {
        message_id: MessageId,
        course_id: TeachingCourseId,
        assignment_ids: Vec<AssignmentId>,
    },
    ManagingAssignment {
        message_id: MessageId,
        course_id: TeachingCourseId,
        assignment_id: AssignmentId,
    },
    AwaitingAssignmentTitle {
        course_id: TeachingCourseId,
    },
    AwaitingAssignmentBody {
        course_id: TeachingCourseId,
        title: String,
    },
    AwaitingAssignmentMaxAttempts {
        course_id: TeachingCourseId,
        title: String,
        body: String,
    },
    AwaitingAssignmentDeadline {
        course_id: TeachingCourseId,
        title: String,
        body: String,
        max_attempts: u16,
    },
    AwaitingDeadlineChange {
        assignment_id: AssignmentId,
    },
    ChoosingAttemptStudent {
        message_id: MessageId,
        assignment_id: AssignmentId,
        student_ids: Vec<UserId>,
    },
    AwaitingAttemptAmount {
        assignment_id: AssignmentId,
        student_id: UserId,
    },
}
