use chrono::{DateTime, Utc};
use teloxide::dispatching::dialogue::{Dialogue, InMemStorage};
use teloxide::types::MessageId;

use super::assignment_form::{AssignmentText, DraftAssignment, InputMode, TermsForm, TermsStep};
use super::makeup_form::{MakeupForm, MakeupStep};
use super::views::MenuAction;
use crate::domain::{
    AssignmentId, Grading, InviteGeneration, LatePenalty, ReviewDecision, ReviewScore,
    StoredFileId, SubmissionId, TeachingCourseId, TelegramUserId, UserId,
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

    pub(super) fn awaiting_review_comment(
        &self,
        submission_id: SubmissionId,
    ) -> Option<SubmissionId> {
        match self {
            Self::Flow(FlowState::AwaitingReviewComment {
                submission_id: current,
                ..
            }) if *current == submission_id => Some(submission_id),
            _ => None,
        }
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

    pub(super) fn makeup_form(self, message_id: MessageId, step: MakeupStep) -> Option<MakeupForm> {
        match self {
            Self::Flow(FlowState::ConfiguringMakeup(form))
                if form.message_id == Some(message_id)
                    && form.step == step
                    && form.mode != InputMode::Manual =>
            {
                Some(form)
            }
            _ => None,
        }
    }

    pub(super) fn active_makeup(&self) -> Option<&MakeupForm> {
        match self {
            Self::Flow(FlowState::ConfiguringMakeup(form)) => Some(form),
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
    GroupInvite {
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
        makeup: bool,
    },
    AwaitingReviewDecision {
        submission_id: SubmissionId,
        target: ReviewTarget,
    },
    AwaitingReviewScore {
        submission_id: SubmissionId,
        max_score: u16,
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

        assert_eq!(state.awaiting_review_comment(current), Some(current));
        assert!(state.awaiting_review_comment(other).is_none());
        assert!(BotState::Idle.awaiting_review_comment(current).is_none());
        assert!(
            BotState::Flow(FlowState::AwaitingReviewDecision {
                submission_id: current,
                target: ReviewTarget::Regular {
                    grading: Grading::PassFail,
                    late: None,
                },
            })
            .awaiting_review_comment(current)
            .is_none()
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
}
