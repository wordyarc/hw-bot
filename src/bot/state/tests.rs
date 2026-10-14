use super::*;
use crate::app::MakeupQuestion;
use crate::bot::assignment_form::AssignmentTerms;
use crate::domain::DeductedFrom;
use crate::test_support::{invite_codec, t0};

fn invite() -> VerifiedInvite {
    let codec = invite_codec();
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
fn a_makeup_prompt_covers_only_work_sent_after_it() {
    let prompted_at = t0();

    assert!(MakeupConsent::PromptedAt(prompted_at).covers(prompted_at));
    assert!(
        MakeupConsent::PromptedAt(prompted_at).covers(prompted_at + chrono::Duration::seconds(1))
    );
    assert!(
        !MakeupConsent::PromptedAt(prompted_at).covers(prompted_at - chrono::Duration::seconds(1))
    );
    assert!(MakeupConsent::Given.covers(prompted_at - chrono::Duration::days(1)));
    assert!(!MakeupConsent::Withheld.covers(prompted_at));
}

#[test]
fn the_start_button_of_the_current_prompt_restarts_it() {
    let (course_id, assignment_id) = (TeachingCourseId::new(), AssignmentId::new());
    let title = BotState::Flow(FlowState::AwaitingAssignmentTitle { course_id });
    let submission = BotState::Flow(FlowState::AwaitingSubmission {
        assignment_id,
        makeup: MakeupConsent::Given,
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
        makeup: MakeupConsent::Withheld,
    });

    assert!(
        !title.is_restarted_by(ReplayableAction::StartAssignmentCreation(
            TeachingCourseId::new()
        ))
    );
    assert!(!submission.is_restarted_by(ReplayableAction::StartSubmission(AssignmentId::new())));
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
        form: terms_form(TermsStep::Deadline),
        action: MenuAction::Profile,
        telegram_user_id: TelegramUserId::new(1).unwrap(),
    });

    assert!(matches!(exiting.active_form(), Some(ActiveForm::Terms(_))));
    assert!(matches!(
        BotState::Flow(FlowState::ConfiguringMakeup(makeup_form(MakeupStep::Date))).active_form(),
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
        makeup: MakeupConsent::Withheld,
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
            makeup: MakeupConsent::Withheld,
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
