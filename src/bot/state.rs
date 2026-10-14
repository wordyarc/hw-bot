use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::{KeyboardMarkup, MessageId};

use super::assignment_form::{AssignmentText, DraftAssignment, IssuedGroups, TermsForm, TermsStep};
use super::callbacks::ReplayableAction;
use super::form::InputMode;
use super::makeup_form::{MakeupForm, MakeupStep};
use super::texts;
use super::views::{MenuAction, student_keyboard, teacher_keyboard};
use crate::domain::{
    AssignmentId, Grading, InviteGeneration, LatePenalty, MaxScore, ReviewDecision, ReviewScore,
    StoredFileId, StudentGroupId, SubmissionId, TeachingCourseId, TelegramUserId, UserId,
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

#[derive(Clone, Copy)]
pub(super) enum ActiveForm<'a> {
    Terms(&'a TermsForm),
    Makeup(&'a MakeupForm),
}

/// Кнопка допустима при совпадении двух условий: ожидаемого состояния диалога и ID сообщения.
/// Оба условия проверяются в каждом методе разбора.
impl BotState {
    pub(super) fn is_restarted_by(&self, action: ReplayableAction) -> bool {
        match (self, action) {
            (
                Self::Flow(FlowState::AwaitingAssignmentTitle { course_id }),
                ReplayableAction::StartAssignmentCreation(started),
            ) => *course_id == started,
            (
                Self::Flow(FlowState::AwaitingSubmission { assignment_id, .. }),
                ReplayableAction::StartSubmission(started),
            ) => *assignment_id == started,
            _ => false,
        }
    }

    pub(super) fn is_managing_assignment(
        &self,
        message_id: MessageId,
        assignment_id: AssignmentId,
    ) -> bool {
        matches!(
            self,
            Self::Screen(ScreenState::ManagingAssignment {
                message_id: current_message,
                assignment_id: current_assignment,
            }) if *current_message == message_id && *current_assignment == assignment_id
        )
    }

    pub(super) fn course_invite(
        &self,
        message_id: MessageId,
    ) -> Option<(TeachingCourseId, InviteGeneration)> {
        match self {
            Self::Screen(ScreenState::CourseInvite {
                message_id: current,
                course_id,
                generation,
            }) if *current == message_id => Some((*course_id, *generation)),
            _ => None,
        }
    }

    pub(super) fn choosing_registration_group(
        &self,
        message_id: MessageId,
    ) -> Option<VerifiedInvite> {
        match self {
            Self::Flow(FlowState::ChoosingRegistrationGroup {
                invite,
                message_id: current,
            }) if *current == message_id => Some(*invite),
            _ => None,
        }
    }

    pub(super) fn choosing_assignment_groups(
        self,
        message_id: MessageId,
    ) -> Option<(AssignmentText, BTreeSet<StudentGroupId>)> {
        match self {
            Self::Flow(FlowState::ChoosingAssignmentGroups {
                message_id: current,
                text,
                selected,
            }) if current == message_id => Some((text, selected)),
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

    pub(super) fn review_decision_target(
        &self,
        submission_id: SubmissionId,
    ) -> Option<ReviewTarget> {
        match self {
            Self::Flow(FlowState::AwaitingReviewDecision {
                submission_id: current,
                target,
            }) if *current == submission_id => Some(*target),
            Self::Flow(FlowState::AwaitingReviewScore {
                submission_id: current,
                max_score,
                late,
            }) if *current == submission_id => Some(ReviewTarget::Regular {
                grading: Grading::Score {
                    max_score: *max_score,
                },
                late: *late,
            }),
            _ => None,
        }
    }

    pub(super) fn awaiting_review_score(
        &self,
        submission_id: SubmissionId,
    ) -> Option<(MaxScore, Option<LatePenalty>)> {
        match self {
            Self::Flow(FlowState::AwaitingReviewScore {
                submission_id: current,
                max_score,
                late,
            }) if *current == submission_id => Some((*max_score, *late)),
            _ => None,
        }
    }

    pub(super) fn is_awaiting_review_comment(&self, submission_id: SubmissionId) -> bool {
        matches!(
            self,
            Self::Flow(FlowState::AwaitingReviewComment {
                submission_id: current,
                ..
            }) if *current == submission_id
        )
    }

    /// ID студента принимается только из показанного списка, чтобы отсечь чужие кнопки.
    pub(super) fn choosing_student(
        &self,
        message_id: MessageId,
        student_id: UserId,
    ) -> Option<(StudentChoice, StudentList)> {
        let (purpose, list) = match self {
            Self::Flow(FlowState::ChoosingStudent {
                message_id,
                purpose,
                student_ids,
            }) => (
                *purpose,
                StudentList {
                    message_id: *message_id,
                    student_ids: student_ids.clone(),
                },
            ),
            Self::Screen(ScreenState::StudentGrades {
                course_id,
                list: Some(list),
                ..
            }) => (
                StudentChoice::Grades {
                    course_id: *course_id,
                },
                list.clone(),
            ),
            _ => return None,
        };
        (list.message_id == message_id && list.student_ids.contains(&student_id))
            .then_some((purpose, list))
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
    ) -> Option<(AssignmentText, IssuedGroups)> {
        match self {
            Self::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: current,
                text,
                groups,
            }) if current == message_id => Some((text, groups)),
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

    pub(super) fn current_terms_form(self, message_id: MessageId) -> Option<TermsForm> {
        match self {
            Self::Flow(FlowState::ConfiguringTerms(form))
                if form.message_id == Some(message_id) && form.mode != InputMode::Manual =>
            {
                Some(form)
            }
            _ => None,
        }
    }

    pub(super) fn terms_form(self, message_id: MessageId, step: TermsStep) -> Option<TermsForm> {
        self.current_terms_form(message_id)
            .filter(|form| form.step == step)
    }

    pub(super) fn current_makeup_form(self, message_id: MessageId) -> Option<MakeupForm> {
        match self {
            Self::Flow(FlowState::ConfiguringMakeup(form))
                if form.message_id == Some(message_id) && form.mode != InputMode::Manual =>
            {
                Some(form)
            }
            _ => None,
        }
    }

    pub(super) fn makeup_form(self, message_id: MessageId, step: MakeupStep) -> Option<MakeupForm> {
        self.current_makeup_form(message_id)
            .filter(|form| form.step == step)
    }

    pub(super) fn active_makeup(&self) -> Option<&MakeupForm> {
        match self {
            Self::Flow(FlowState::ConfiguringMakeup(form)) => Some(form),
            _ => None,
        }
    }

    pub(super) fn active_form(&self) -> Option<ActiveForm<'_>> {
        match self {
            Self::Flow(
                FlowState::ConfiguringTerms(form) | FlowState::ConfirmingTermsExit { form, .. },
            ) => Some(ActiveForm::Terms(form)),
            Self::Flow(FlowState::ConfiguringMakeup(form)) => Some(ActiveForm::Makeup(form)),
            _ => None,
        }
    }

    pub(super) fn makeup_grant(&self, message_id: MessageId) -> Option<MakeupGrant> {
        let (current, course_id, student_id, origin) = match self {
            Self::Screen(ScreenState::MakeupReviewed {
                message_id,
                course_id,
                student_id,
                sandbox,
            }) => (
                message_id,
                course_id,
                student_id,
                GrantOrigin::Review { sandbox: *sandbox },
            ),
            Self::Screen(ScreenState::StudentGrades {
                message_id,
                course_id,
                student_id,
                list,
            }) => (
                message_id,
                course_id,
                student_id,
                GrantOrigin::Grades { list: list.clone() },
            ),
            _ => return None,
        };
        (*current == message_id).then_some(MakeupGrant {
            course_id: *course_id,
            student_id: *student_id,
            origin,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum GrantOrigin {
    Review { sandbox: bool },
    Grades { list: Option<StudentList> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MakeupGrant {
    pub course_id: TeachingCourseId,
    pub student_id: UserId,
    pub origin: GrantOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StudentList {
    pub message_id: MessageId,
    pub student_ids: Vec<UserId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReviewTarget {
    Regular {
        grading: Grading,
        late: Option<LatePenalty>,
    },
    Makeup,
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
    CourseInvite {
        message_id: MessageId,
        course_id: TeachingCourseId,
        generation: InviteGeneration,
    },
    SandboxSubmissions {
        message_id: MessageId,
        shown_at: DateTime<Utc>,
    },
    MakeupReviewed {
        message_id: MessageId,
        course_id: TeachingCourseId,
        student_id: UserId,
        sandbox: bool,
    },
    StudentGrades {
        message_id: MessageId,
        course_id: TeachingCourseId,
        student_id: UserId,
        list: Option<StudentList>,
    },
}

#[derive(Clone)]
pub(super) enum FlowState {
    /// После проверки подписи исходный bearer credential больше не хранится.
    ChoosingRegistrationGroup {
        invite: VerifiedInvite,
        message_id: MessageId,
    },
    AwaitingRegistrationName {
        invite: VerifiedInvite,
        group_id: StudentGroupId,
    },
    ConfirmingInviteRotation {
        message_id: MessageId,
        course_id: TeachingCourseId,
        generation: InviteGeneration,
    },
    AwaitingSubmission {
        assignment_id: AssignmentId,
        makeup: bool,
    },
    AwaitingReviewDecision {
        submission_id: SubmissionId,
        target: ReviewTarget,
    },
    AwaitingReviewScore {
        submission_id: SubmissionId,
        max_score: MaxScore,
        late: Option<LatePenalty>,
    },
    AwaitingReviewComment {
        submission_id: SubmissionId,
        decision: ReviewDecision,
        score: Option<ReviewScore>,
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
    ChoosingAssignmentGroups {
        message_id: MessageId,
        text: AssignmentText,
        selected: BTreeSet<StudentGroupId>,
    },
    AwaitingAssignmentMaxAttempts {
        message_id: MessageId,
        text: AssignmentText,
        groups: IssuedGroups,
    },
    ChoosingGrading {
        message_id: MessageId,
        draft: DraftAssignment,
    },
    AwaitingGradingText {
        draft: DraftAssignment,
    },
    ConfiguringTerms(TermsForm),
    ConfiguringMakeup(MakeupForm),
    ConfirmingTermsExit {
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

impl FlowState {
    pub(super) fn abandon_notice(&self) -> Option<(&'static str, KeyboardMarkup)> {
        let notice = match self {
            Self::AwaitingSubmission { .. } => (texts::ACTION_CANCELLED, student_keyboard()),
            Self::AwaitingAssignmentTitle { .. }
            | Self::AwaitingAssignmentBody { .. }
            | Self::ChoosingAssignmentGroups { .. }
            | Self::AwaitingAssignmentMaxAttempts { .. }
            | Self::ChoosingGrading { .. }
            | Self::AwaitingGradingText { .. } => (texts::DRAFT_DISCARDED, teacher_keyboard()),
            Self::AwaitingReviewScore { .. }
            | Self::AwaitingReviewComment { .. }
            | Self::AwaitingBulkReviewComment { .. }
            | Self::AwaitingAssignmentFile { .. }
            | Self::AwaitingAttemptAmount { .. } => (texts::ACTION_CANCELLED, teacher_keyboard()),
            Self::ChoosingRegistrationGroup { .. }
            | Self::AwaitingRegistrationName { .. }
            | Self::ConfirmingInviteRotation { .. }
            | Self::AwaitingReviewDecision { .. }
            | Self::ConfirmingAssignmentFileDeletion { .. }
            | Self::ConfiguringTerms(_)
            | Self::ConfiguringMakeup(_)
            | Self::ConfirmingTermsExit { .. }
            | Self::ChoosingStudent { .. } => return None,
        };
        Some(notice)
    }
}

impl BotState {
    pub(super) fn button_hint(&self) -> Option<&'static str> {
        match self {
            Self::Flow(FlowState::AwaitingReviewDecision { .. }) => {
                Some(texts::CHOOSE_REVIEW_DECISION)
            }
            Self::Screen(ScreenState::ManagingAssignment { .. }) => {
                Some(texts::CHOOSE_ASSIGNMENT_ACTION)
            }
            Self::Screen(
                ScreenState::CourseInvite { .. }
                | ScreenState::SandboxSubmissions { .. }
                | ScreenState::MakeupReviewed { .. }
                | ScreenState::StudentGrades { .. },
            )
            | Self::Flow(FlowState::ConfirmingInviteRotation { .. }) => {
                Some(texts::CHOOSE_ACTION_ABOVE)
            }
            Self::Screen(ScreenState::ManagingAssignmentFiles { .. }) => {
                Some(texts::CHOOSE_FILE_ACTION)
            }
            Self::Flow(FlowState::ConfirmingAssignmentFileDeletion { .. }) => {
                Some(texts::CONFIRM_FILE_DELETION)
            }
            Self::Flow(
                FlowState::ConfirmingTermsExit { .. }
                | FlowState::ChoosingRegistrationGroup { .. }
                | FlowState::ChoosingAssignmentGroups { .. },
            ) => Some(texts::CHOOSE_BUTTON),
            Self::Flow(FlowState::ChoosingStudent { .. }) => Some(texts::CHOOSE_STUDENT),
            Self::Idle
            | Self::Screen(ScreenState::Guest)
            | Self::Flow(
                FlowState::AwaitingRegistrationName { .. }
                | FlowState::AwaitingSubmission { .. }
                | FlowState::AwaitingReviewScore { .. }
                | FlowState::AwaitingReviewComment { .. }
                | FlowState::AwaitingBulkReviewComment { .. }
                | FlowState::AwaitingAssignmentFile { .. }
                | FlowState::AwaitingAssignmentTitle { .. }
                | FlowState::AwaitingAssignmentBody { .. }
                | FlowState::AwaitingAssignmentMaxAttempts { .. }
                | FlowState::ChoosingGrading { .. }
                | FlowState::AwaitingGradingText { .. }
                | FlowState::ConfiguringTerms(_)
                | FlowState::ConfiguringMakeup(_)
                | FlowState::AwaitingAttemptAmount { .. },
            ) => None,
        }
    }
}

#[cfg(test)]
mod tests;
