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
mod tests {
    use super::*;
    use crate::app::MakeupQuestion;
    use crate::bot::assignment_form::AssignmentTerms;
    use crate::domain::DeductedFrom;
    use crate::invites::{CourseInviteCodec, InviteSigningSecret};
    use crate::test_support::t0;

    fn invite() -> VerifiedInvite {
        let codec = CourseInviteCodec::new(
            InviteSigningSecret::from_base64url("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8")
                .unwrap(),
        );
        codec
            .decode(&codec.encode(TeachingCourseId::new(), InviteGeneration::FIRST))
            .unwrap()
    }

    fn text() -> AssignmentText {
        AssignmentText {
            course_id: TeachingCourseId::new(),
            title: "Title".into(),
            body: "Body".into(),
        }
    }

    fn draft() -> DraftAssignment {
        DraftAssignment::new(text(), IssuedGroups::Single(StudentGroupId::new()), 1)
    }

    fn terms_form(step: TermsStep) -> TermsForm {
        let mut form = TermsForm::new(
            AssignmentTerms::creating(draft(), MakeupQuestion::default()),
            step,
        );
        form.message_id = Some(MessageId(10));
        form
    }

    fn makeup_form(step: MakeupStep) -> MakeupForm {
        let mut form = MakeupForm::new(TeachingCourseId::new(), None, Vec::new(), t0());
        form.go(step);
        form.message_id = Some(MessageId(10));
        form
    }

    fn late() -> LatePenalty {
        LatePenalty::restore(2, 2, DeductedFrom::WorkScore).unwrap()
    }

    fn file_deletion() -> FileDeletion {
        FileDeletion {
            assignment_id: AssignmentId::new(),
            file_id: StoredFileId::new(),
            expected_revision: 3,
            creation_flow: false,
        }
    }

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

        assert!(state.is_managing_assignment(MessageId(10), current));
        assert!(!state.is_managing_assignment(MessageId(11), current));
        assert!(!state.is_managing_assignment(MessageId(10), other));
    }

    #[test]
    fn changing_a_decision_requires_the_comment_step_of_the_same_submission() {
        let current = SubmissionId::new();
        let other = SubmissionId::new();
        let state = BotState::Flow(FlowState::AwaitingReviewComment {
            submission_id: current,
            decision: ReviewDecision::Accept,
            score: Some(ReviewScore {
                value: 4,
                max_score: 5,
            }),
        });

        assert!(state.is_awaiting_review_comment(current));
        assert!(!state.is_awaiting_review_comment(other));
        assert!(!BotState::Idle.is_awaiting_review_comment(current));
        assert!(
            !BotState::Flow(FlowState::AwaitingReviewDecision {
                submission_id: current,
                target: ReviewTarget::Regular {
                    grading: Grading::PassFail,
                    late: None,
                },
            })
            .is_awaiting_review_comment(current)
        );
    }

    #[test]
    fn a_makeup_grant_requires_the_current_screen_of_either_origin() {
        let course_id = TeachingCourseId::new();
        let student_id = UserId::new();
        let list = StudentList {
            message_id: MessageId(9),
            student_ids: vec![student_id],
        };
        for (state, origin) in [
            (
                ScreenState::MakeupReviewed {
                    message_id: MessageId(10),
                    course_id,
                    student_id,
                    sandbox: true,
                },
                GrantOrigin::Review { sandbox: true },
            ),
            (
                ScreenState::StudentGrades {
                    message_id: MessageId(10),
                    course_id,
                    student_id,
                    list: Some(list.clone()),
                },
                GrantOrigin::Grades {
                    list: Some(list.clone()),
                },
            ),
        ] {
            let state = BotState::Screen(state);
            assert_eq!(
                state.makeup_grant(MessageId(10)),
                Some(MakeupGrant {
                    course_id,
                    student_id,
                    origin,
                })
            );
            assert!(state.makeup_grant(MessageId(11)).is_none());
        }
        assert!(BotState::Idle.makeup_grant(MessageId(10)).is_none());
    }

    #[test]
    fn the_student_list_stays_usable_under_an_opened_grades_card() {
        let course_id = TeachingCourseId::new();
        let (shown, other) = (UserId::new(), UserId::new());
        let list = StudentList {
            message_id: MessageId(9),
            student_ids: vec![shown, other],
        };
        let card = BotState::Screen(ScreenState::StudentGrades {
            message_id: MessageId(10),
            course_id,
            student_id: shown,
            list: Some(list.clone()),
        });

        let Some((StudentChoice::Grades { course_id: chosen }, kept)) =
            card.choosing_student(MessageId(9), other)
        else {
            panic!("the list under the card must stay usable");
        };
        assert_eq!((chosen, kept), (course_id, list));
        assert!(card.choosing_student(MessageId(10), other).is_none());
        assert!(card.choosing_student(MessageId(9), UserId::new()).is_none());

        let without_list = BotState::Screen(ScreenState::StudentGrades {
            message_id: MessageId(10),
            course_id,
            student_id: shown,
            list: None,
        });
        assert!(without_list.choosing_student(MessageId(9), other).is_none());
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

    #[test]
    fn the_start_button_of_the_current_prompt_restarts_it() {
        let (course_id, assignment_id) = (TeachingCourseId::new(), AssignmentId::new());
        let title = BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id });
        let submission = BotState::Flow(FlowState::AwaitingSubmission {
            assignment_id,
            makeup: true,
        });

        assert!(title.is_restarted_by(ReplayableAction::StartAssignmentCreation(course_id)));
        assert!(submission.is_restarted_by(ReplayableAction::StartSubmission(assignment_id)));
    }

    #[test]
    fn a_start_button_of_another_target_or_flow_does_not_restart_the_prompt() {
        let (course_id, assignment_id) = (TeachingCourseId::new(), AssignmentId::new());
        let title = BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id });
        let submission = BotState::Flow(FlowState::AwaitingSubmission {
            assignment_id,
            makeup: false,
        });

        assert!(
            !title.is_restarted_by(ReplayableAction::StartAssignmentCreation(
                TeachingCourseId::new()
            ))
        );
        assert!(
            !submission.is_restarted_by(ReplayableAction::StartSubmission(AssignmentId::new()))
        );
        assert!(!title.is_restarted_by(ReplayableAction::CourseInvite(course_id)));
        assert!(!submission.is_restarted_by(ReplayableAction::Assignment(assignment_id)));
        assert!(!BotState::Idle.is_restarted_by(ReplayableAction::StartSubmission(assignment_id)));
    }

    #[test]
    fn the_invite_screen_requires_its_current_message() {
        let course_id = TeachingCourseId::new();
        let generation = InviteGeneration::FIRST;
        let state = BotState::Screen(ScreenState::CourseInvite {
            message_id: MessageId(10),
            course_id,
            generation,
        });
        let confirming = BotState::Flow(FlowState::ConfirmingInviteRotation {
            message_id: MessageId(10),
            course_id,
            generation,
        });

        assert_eq!(
            state.course_invite(MessageId(10)),
            Some((course_id, generation))
        );
        assert!(state.course_invite(MessageId(11)).is_none());
        assert!(confirming.course_invite(MessageId(10)).is_none());
    }

    #[test]
    fn an_invite_rotation_requires_its_current_confirmation() {
        let course_id = TeachingCourseId::new();
        let generation = InviteGeneration::FIRST;
        let state = BotState::Flow(FlowState::ConfirmingInviteRotation {
            message_id: MessageId(10),
            course_id,
            generation,
        });
        let screen = BotState::Screen(ScreenState::CourseInvite {
            message_id: MessageId(10),
            course_id,
            generation,
        });

        assert_eq!(
            state.confirming_invite_rotation(MessageId(10)),
            Some((course_id, generation))
        );
        assert!(state.confirming_invite_rotation(MessageId(11)).is_none());
        assert!(screen.confirming_invite_rotation(MessageId(10)).is_none());
    }

    #[test]
    fn a_registration_group_comes_from_the_current_group_choice() {
        let invite = invite();
        let state = BotState::Flow(FlowState::ChoosingRegistrationGroup {
            invite,
            message_id: MessageId(10),
        });
        let naming = BotState::Flow(FlowState::AwaitingRegistrationName {
            invite,
            group_id: StudentGroupId::new(),
        });

        assert_eq!(
            state.choosing_registration_group(MessageId(10)),
            Some(invite)
        );
        assert!(state.choosing_registration_group(MessageId(11)).is_none());
        assert!(naming.choosing_registration_group(MessageId(10)).is_none());
    }

    #[test]
    fn assignment_groups_are_toggled_on_the_current_group_choice() {
        let selected = BTreeSet::from([StudentGroupId::new()]);
        let state = || {
            BotState::Flow(FlowState::ChoosingAssignmentGroups {
                message_id: MessageId(10),
                text: text(),
                selected: selected.clone(),
            })
        };

        let (shown, chosen) = state().choosing_assignment_groups(MessageId(10)).unwrap();
        assert_eq!((shown.title.as_str(), chosen), ("Title", selected.clone()));
        assert!(state().choosing_assignment_groups(MessageId(11)).is_none());
        assert!(
            BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: MessageId(10),
                text: text(),
                groups: IssuedGroups::Single(StudentGroupId::new()),
            })
            .choosing_assignment_groups(MessageId(10))
            .is_none()
        );
    }

    #[test]
    fn a_file_deletion_is_confirmed_only_from_its_current_question() {
        let deletion = file_deletion();
        let state = BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion {
            message_id: MessageId(10),
            deletion,
        });
        let files = BotState::Screen(ScreenState::ManagingAssignmentFiles {
            message_id: MessageId(10),
            assignment_id: deletion.assignment_id,
            revision: deletion.expected_revision,
            file_ids: vec![deletion.file_id],
            deletion_requires_confirmation: true,
            creation_flow: false,
        });

        let confirmed = state.confirming_file_deletion(MessageId(10)).unwrap();
        assert_eq!(
            (
                confirmed.assignment_id,
                confirmed.file_id,
                confirmed.expected_revision
            ),
            (
                deletion.assignment_id,
                deletion.file_id,
                deletion.expected_revision
            )
        );
        assert!(state.confirming_file_deletion(MessageId(11)).is_none());
        assert!(files.confirming_file_deletion(MessageId(10)).is_none());
    }

    #[test]
    fn the_decision_step_keeps_its_target_for_the_same_submission() {
        let submission_id = SubmissionId::new();
        let state = BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id,
            target: ReviewTarget::Makeup,
        });

        assert_eq!(
            state.review_decision_target(submission_id),
            Some(ReviewTarget::Makeup)
        );
        assert!(state.review_decision_target(SubmissionId::new()).is_none());
        assert!(
            BotState::Flow(FlowState::AwaitingReviewComment {
                submission_id,
                decision: ReviewDecision::Reject,
                score: None,
            })
            .review_decision_target(submission_id)
            .is_none()
        );
    }

    #[test]
    fn a_decision_changed_while_the_score_is_awaited_keeps_the_scale_and_lateness() {
        let submission_id = SubmissionId::new();
        let max_score = MaxScore::new(10).unwrap();
        let state = BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            late: Some(late()),
        });

        assert_eq!(
            state.review_decision_target(submission_id),
            Some(ReviewTarget::Regular {
                grading: Grading::Score { max_score },
                late: Some(late()),
            })
        );
        assert!(state.review_decision_target(SubmissionId::new()).is_none());
    }

    #[test]
    fn a_score_is_taken_only_while_the_same_submission_awaits_it() {
        let submission_id = SubmissionId::new();
        let max_score = MaxScore::new(10).unwrap();
        let state = BotState::Flow(FlowState::AwaitingReviewScore {
            submission_id,
            max_score,
            late: Some(late()),
        });
        let deciding = BotState::Flow(FlowState::AwaitingReviewDecision {
            submission_id,
            target: ReviewTarget::Regular {
                grading: Grading::Score { max_score },
                late: None,
            },
        });

        assert_eq!(
            state.awaiting_review_score(submission_id),
            Some((max_score, Some(late())))
        );
        assert!(state.awaiting_review_score(SubmissionId::new()).is_none());
        assert!(deciding.awaiting_review_score(submission_id).is_none());
    }

    #[test]
    fn an_extra_attempt_student_must_come_from_the_shown_list() {
        let assignment_id = AssignmentId::new();
        let (shown, other) = (UserId::new(), UserId::new());
        let state = BotState::Flow(FlowState::ChoosingStudent {
            message_id: MessageId(10),
            purpose: StudentChoice::ExtraAttempts { assignment_id },
            student_ids: vec![shown],
        });

        let Some((
            StudentChoice::ExtraAttempts {
                assignment_id: chosen,
            },
            list,
        )) = state.choosing_student(MessageId(10), shown)
        else {
            panic!("a listed student must be chosen for extra attempts");
        };
        assert_eq!(
            (chosen, list),
            (
                assignment_id,
                StudentList {
                    message_id: MessageId(10),
                    student_ids: vec![shown],
                }
            )
        );
        assert!(state.choosing_student(MessageId(11), shown).is_none());
        assert!(state.choosing_student(MessageId(10), other).is_none());
    }

    #[test]
    fn an_attempt_amount_requires_the_current_amount_question() {
        let (assignment_id, student_id) = (AssignmentId::new(), UserId::new());
        let state = BotState::Flow(FlowState::AwaitingAttemptAmount {
            message_id: MessageId(10),
            assignment_id,
            student_id,
        });
        let choosing = BotState::Flow(FlowState::ChoosingStudent {
            message_id: MessageId(10),
            purpose: StudentChoice::ExtraAttempts { assignment_id },
            student_ids: vec![student_id],
        });

        assert_eq!(
            state.awaiting_attempt_amount(MessageId(10)),
            Some((assignment_id, student_id))
        );
        assert!(state.awaiting_attempt_amount(MessageId(11)).is_none());
        assert!(choosing.awaiting_attempt_amount(MessageId(10)).is_none());
    }

    #[test]
    fn max_attempts_require_the_current_attempts_question() {
        let group_id = StudentGroupId::new();
        let state = || {
            BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: MessageId(10),
                text: text(),
                groups: IssuedGroups::Single(group_id),
            })
        };

        let (shown, groups) = state().awaiting_max_attempts(MessageId(10)).unwrap();
        assert_eq!(
            (shown.title.as_str(), groups.ids()),
            ("Title", BTreeSet::from([group_id]))
        );
        assert!(state().awaiting_max_attempts(MessageId(11)).is_none());
        assert!(
            BotState::Flow(FlowState::ChoosingGrading {
                message_id: MessageId(10),
                draft: draft(),
            })
            .awaiting_max_attempts(MessageId(10))
            .is_none()
        );
    }

    #[test]
    fn a_grading_is_chosen_only_on_the_current_grading_question() {
        let state = || {
            BotState::Flow(FlowState::ChoosingGrading {
                message_id: MessageId(10),
                draft: draft(),
            })
        };

        let chosen = state().choosing_grading(MessageId(10)).unwrap();
        assert_eq!(chosen.text.title, "Title");
        assert!(state().choosing_grading(MessageId(11)).is_none());
        assert!(
            BotState::Flow(FlowState::AwaitingGradingText { draft: draft() })
                .choosing_grading(MessageId(10))
                .is_none()
        );
    }

    #[test]
    fn terms_buttons_require_the_current_form_message() {
        let state = || BotState::Flow(FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline)));
        let exiting = BotState::Flow(FlowState::ConfirmingTermsExit {
            message_id: MessageId(10),
            form: terms_form(TermsStep::Deadline),
            action: MenuAction::Profile,
            telegram_user_id: TelegramUserId::new(1).unwrap(),
        });

        assert_eq!(
            state()
                .current_terms_form(MessageId(10))
                .map(|form| form.step),
            Some(TermsStep::Deadline)
        );
        assert!(state().current_terms_form(MessageId(11)).is_none());
        assert!(exiting.current_terms_form(MessageId(10)).is_none());
    }

    #[test]
    fn a_terms_form_waiting_for_typed_input_ignores_its_buttons() {
        let form = terms_form(TermsStep::Period);
        assert_eq!(form.mode, InputMode::Manual);

        assert!(
            BotState::Flow(FlowState::ConfiguringTerms(form))
                .current_terms_form(MessageId(10))
                .is_none()
        );
    }

    #[test]
    fn a_terms_button_of_another_step_is_stale() {
        let state = || BotState::Flow(FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline)));

        assert!(
            state()
                .terms_form(MessageId(10), TermsStep::Deadline)
                .is_some()
        );
        assert!(
            state()
                .terms_form(MessageId(10), TermsStep::Summary)
                .is_none()
        );
    }

    #[test]
    fn makeup_buttons_require_the_current_form_message() {
        let state = || BotState::Flow(FlowState::ConfiguringMakeup(makeup_form(MakeupStep::Date)));

        assert_eq!(
            state()
                .current_makeup_form(MessageId(10))
                .map(|form| form.step),
            Some(MakeupStep::Date)
        );
        assert!(state().current_makeup_form(MessageId(11)).is_none());
        assert!(
            BotState::Flow(FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline)))
                .current_makeup_form(MessageId(10))
                .is_none()
        );
    }

    #[test]
    fn a_makeup_form_waiting_for_typed_input_ignores_its_buttons() {
        let mut form = makeup_form(MakeupStep::Attempts);
        form.mode = InputMode::Manual;

        assert!(
            BotState::Flow(FlowState::ConfiguringMakeup(form))
                .current_makeup_form(MessageId(10))
                .is_none()
        );
    }

    #[test]
    fn a_makeup_button_of_another_step_is_stale() {
        let state = || {
            BotState::Flow(FlowState::ConfiguringMakeup(makeup_form(
                MakeupStep::Attempts,
            )))
        };

        assert!(
            state()
                .makeup_form(MessageId(10), MakeupStep::Attempts)
                .is_some()
        );
        assert!(
            state()
                .makeup_form(MessageId(10), MakeupStep::Date)
                .is_none()
        );
    }

    #[test]
    fn a_makeup_form_stays_active_whatever_its_input_mode() {
        let mut form = makeup_form(MakeupStep::Attempts);
        form.mode = InputMode::Manual;

        assert!(
            BotState::Flow(FlowState::ConfiguringMakeup(form))
                .active_makeup()
                .is_some()
        );
        assert!(
            BotState::Flow(FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline)))
                .active_makeup()
                .is_none()
        );
    }

    #[test]
    fn the_terms_exit_question_keeps_the_terms_form_active() {
        let exiting = BotState::Flow(FlowState::ConfirmingTermsExit {
            message_id: MessageId(10),
            form: terms_form(TermsStep::Deadline),
            action: MenuAction::Profile,
            telegram_user_id: TelegramUserId::new(1).unwrap(),
        });

        assert!(matches!(exiting.active_form(), Some(ActiveForm::Terms(_))));
        assert!(matches!(
            BotState::Flow(FlowState::ConfiguringMakeup(makeup_form(MakeupStep::Date)))
                .active_form(),
            Some(ActiveForm::Makeup(_))
        ));
        assert!(
            BotState::Flow(FlowState::AwaitingAssignmentTitle {
                course_id: TeachingCourseId::new(),
            })
            .active_form()
            .is_none()
        );
    }

    #[test]
    fn abandoning_a_submission_prompt_restores_the_student_menu() {
        let flow = FlowState::AwaitingSubmission {
            assignment_id: AssignmentId::new(),
            makeup: false,
        };

        assert_eq!(
            flow.abandon_notice(),
            Some((texts::ACTION_CANCELLED, student_keyboard()))
        );
    }

    #[test]
    fn abandoning_assignment_creation_discards_the_draft() {
        let course_id = TeachingCourseId::new();
        let flows = [
            FlowState::AwaitingAssignmentTitle { course_id },
            FlowState::AwaitingAssignmentBody {
                course_id,
                title: "Title".into(),
            },
            FlowState::ChoosingAssignmentGroups {
                message_id: MessageId(10),
                text: text(),
                selected: BTreeSet::new(),
            },
            FlowState::AwaitingAssignmentMaxAttempts {
                message_id: MessageId(10),
                text: text(),
                groups: IssuedGroups::Single(StudentGroupId::new()),
            },
            FlowState::ChoosingGrading {
                message_id: MessageId(10),
                draft: draft(),
            },
            FlowState::AwaitingGradingText { draft: draft() },
        ];

        for (index, flow) in flows.iter().enumerate() {
            assert_eq!(
                flow.abandon_notice(),
                Some((texts::DRAFT_DISCARDED, teacher_keyboard())),
                "{index}"
            );
        }
    }

    #[test]
    fn abandoning_a_teacher_text_step_cancels_it_with_the_teacher_menu() {
        let flows = [
            FlowState::AwaitingReviewScore {
                submission_id: SubmissionId::new(),
                max_score: MaxScore::new(10).unwrap(),
                late: None,
            },
            FlowState::AwaitingReviewComment {
                submission_id: SubmissionId::new(),
                decision: ReviewDecision::Reject,
                score: None,
            },
            FlowState::AwaitingBulkReviewComment {
                decision: ReviewDecision::Accept,
                cutoff: t0(),
            },
            FlowState::AwaitingAssignmentFile {
                assignment_id: AssignmentId::new(),
                expected_revision: 1,
                creation_flow: false,
            },
            FlowState::AwaitingAttemptAmount {
                message_id: MessageId(10),
                assignment_id: AssignmentId::new(),
                student_id: UserId::new(),
            },
        ];

        for (index, flow) in flows.iter().enumerate() {
            assert_eq!(
                flow.abandon_notice(),
                Some((texts::ACTION_CANCELLED, teacher_keyboard())),
                "{index}"
            );
        }
    }

    #[test]
    fn registration_review_decisions_confirmations_and_forms_are_abandoned_silently() {
        let flows = [
            FlowState::ChoosingRegistrationGroup {
                invite: invite(),
                message_id: MessageId(10),
            },
            FlowState::AwaitingRegistrationName {
                invite: invite(),
                group_id: StudentGroupId::new(),
            },
            FlowState::ConfirmingInviteRotation {
                message_id: MessageId(10),
                course_id: TeachingCourseId::new(),
                generation: InviteGeneration::FIRST,
            },
            FlowState::AwaitingReviewDecision {
                submission_id: SubmissionId::new(),
                target: ReviewTarget::Makeup,
            },
            FlowState::ConfirmingAssignmentFileDeletion {
                message_id: MessageId(10),
                deletion: file_deletion(),
            },
            FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline)),
            FlowState::ConfiguringMakeup(makeup_form(MakeupStep::Date)),
            FlowState::ConfirmingTermsExit {
                message_id: MessageId(10),
                form: terms_form(TermsStep::Deadline),
                action: MenuAction::Profile,
                telegram_user_id: TelegramUserId::new(1).unwrap(),
            },
            FlowState::ChoosingStudent {
                message_id: MessageId(10),
                purpose: StudentChoice::ExtraAttempts {
                    assignment_id: AssignmentId::new(),
                },
                student_ids: Vec::new(),
            },
        ];

        for (index, flow) in flows.iter().enumerate() {
            assert_eq!(flow.abandon_notice(), None, "{index}");
        }
    }

    #[test]
    fn typed_text_on_a_button_screen_is_answered_with_a_hint_naming_its_buttons() {
        let course_id = TeachingCourseId::new();
        let cases = [
            (
                BotState::Flow(FlowState::AwaitingReviewDecision {
                    submission_id: SubmissionId::new(),
                    target: ReviewTarget::Makeup,
                }),
                texts::CHOOSE_REVIEW_DECISION,
            ),
            (
                BotState::Screen(ScreenState::ManagingAssignment {
                    message_id: MessageId(10),
                    assignment_id: AssignmentId::new(),
                }),
                texts::CHOOSE_ASSIGNMENT_ACTION,
            ),
            (
                BotState::Screen(ScreenState::CourseInvite {
                    message_id: MessageId(10),
                    course_id,
                    generation: InviteGeneration::FIRST,
                }),
                texts::CHOOSE_ACTION_ABOVE,
            ),
            (
                BotState::Screen(ScreenState::SandboxSubmissions {
                    message_id: MessageId(10),
                    shown_at: t0(),
                }),
                texts::CHOOSE_ACTION_ABOVE,
            ),
            (
                BotState::Screen(ScreenState::MakeupReviewed {
                    message_id: MessageId(10),
                    course_id,
                    student_id: UserId::new(),
                    sandbox: false,
                }),
                texts::CHOOSE_ACTION_ABOVE,
            ),
            (
                BotState::Screen(ScreenState::StudentGrades {
                    message_id: MessageId(10),
                    course_id,
                    student_id: UserId::new(),
                    list: None,
                }),
                texts::CHOOSE_ACTION_ABOVE,
            ),
            (
                BotState::Flow(FlowState::ConfirmingInviteRotation {
                    message_id: MessageId(10),
                    course_id,
                    generation: InviteGeneration::FIRST,
                }),
                texts::CHOOSE_ACTION_ABOVE,
            ),
            (
                BotState::Screen(ScreenState::ManagingAssignmentFiles {
                    message_id: MessageId(10),
                    assignment_id: AssignmentId::new(),
                    revision: 1,
                    file_ids: Vec::new(),
                    deletion_requires_confirmation: false,
                    creation_flow: false,
                }),
                texts::CHOOSE_FILE_ACTION,
            ),
            (
                BotState::Flow(FlowState::ConfirmingAssignmentFileDeletion {
                    message_id: MessageId(10),
                    deletion: file_deletion(),
                }),
                texts::CONFIRM_FILE_DELETION,
            ),
            (
                BotState::Flow(FlowState::ConfirmingTermsExit {
                    message_id: MessageId(10),
                    form: terms_form(TermsStep::Deadline),
                    action: MenuAction::Profile,
                    telegram_user_id: TelegramUserId::new(1).unwrap(),
                }),
                texts::CHOOSE_BUTTON,
            ),
            (
                BotState::Flow(FlowState::ChoosingRegistrationGroup {
                    invite: invite(),
                    message_id: MessageId(10),
                }),
                texts::CHOOSE_BUTTON,
            ),
            (
                BotState::Flow(FlowState::ChoosingAssignmentGroups {
                    message_id: MessageId(10),
                    text: text(),
                    selected: BTreeSet::new(),
                }),
                texts::CHOOSE_BUTTON,
            ),
            (
                BotState::Flow(FlowState::ChoosingStudent {
                    message_id: MessageId(10),
                    purpose: StudentChoice::Grades { course_id },
                    student_ids: Vec::new(),
                }),
                texts::CHOOSE_STUDENT,
            ),
        ];

        for (index, (state, hint)) in cases.iter().enumerate() {
            assert_eq!(state.button_hint(), Some(*hint), "{index}");
        }
    }

    #[test]
    fn idle_and_typed_input_states_have_no_button_hint() {
        let states = [
            BotState::Idle,
            BotState::Screen(ScreenState::Guest),
            BotState::Flow(FlowState::AwaitingRegistrationName {
                invite: invite(),
                group_id: StudentGroupId::new(),
            }),
            BotState::Flow(FlowState::AwaitingSubmission {
                assignment_id: AssignmentId::new(),
                makeup: false,
            }),
            BotState::Flow(FlowState::AwaitingReviewScore {
                submission_id: SubmissionId::new(),
                max_score: MaxScore::new(10).unwrap(),
                late: None,
            }),
            BotState::Flow(FlowState::AwaitingReviewComment {
                submission_id: SubmissionId::new(),
                decision: ReviewDecision::Accept,
                score: None,
            }),
            BotState::Flow(FlowState::AwaitingBulkReviewComment {
                decision: ReviewDecision::Reject,
                cutoff: t0(),
            }),
            BotState::Flow(FlowState::AwaitingAssignmentFile {
                assignment_id: AssignmentId::new(),
                expected_revision: 1,
                creation_flow: true,
            }),
            BotState::Flow(FlowState::AwaitingAssignmentTitle {
                course_id: TeachingCourseId::new(),
            }),
            BotState::Flow(FlowState::AwaitingAssignmentBody {
                course_id: TeachingCourseId::new(),
                title: "Title".into(),
            }),
            BotState::Flow(FlowState::AwaitingAssignmentMaxAttempts {
                message_id: MessageId(10),
                text: text(),
                groups: IssuedGroups::Single(StudentGroupId::new()),
            }),
            BotState::Flow(FlowState::ChoosingGrading {
                message_id: MessageId(10),
                draft: draft(),
            }),
            BotState::Flow(FlowState::AwaitingGradingText { draft: draft() }),
            BotState::Flow(FlowState::ConfiguringTerms(terms_form(TermsStep::Deadline))),
            BotState::Flow(FlowState::ConfiguringMakeup(makeup_form(MakeupStep::Date))),
            BotState::Flow(FlowState::AwaitingAttemptAmount {
                message_id: MessageId(10),
                assignment_id: AssignmentId::new(),
                student_id: UserId::new(),
            }),
        ];

        for (index, state) in states.iter().enumerate() {
            assert_eq!(state.button_hint(), None, "{index}");
        }
    }
}
