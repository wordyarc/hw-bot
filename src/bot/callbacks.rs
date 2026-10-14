use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::NaiveDate;
use uuid::Uuid;

use super::handlers::input::parse_u16;
use crate::domain::{
    AssignmentId, DeductedFrom, DomainResult, Grading, MakeupTerms, ReviewDecision, StudentGroupId,
    SubmissionId, TeachingCourseId, UserId,
};

use CallbackAction::{Flow, Informational, Replayable, Screen};

pub const MENU_ASSIGNMENTS: &str = "menu:assignments";
pub const PAGE_PREFIX: &str = "pg:";
pub const MENU_ARCHIVE: &str = "menu:archive";
pub const MENU_QUEUE: &str = "menu:queue";
pub const MENU_PROFILE: &str = "menu:profile";
pub const MENU_COURSES: &str = "menu:courses";
pub const MENU_SANDBOX: &str = "menu:sandbox";
pub const NEXT_REVIEW: &str = "review:next";
pub const REVIEW_HUB: &str = "review:hub";
pub const NEXT_MAKEUP_REVIEW: &str = "review:makeup";

pub const COURSE_PREFIX: &str = "course:";
pub const ASSIGNMENT_PREFIX: &str = "assignment:";
pub const MANAGE_PREFIX: &str = "manage:";
pub const CREATE_PREFIX: &str = "create:";
pub const SUBMIT_PREFIX: &str = "submit:";
pub const FILES_PREFIX: &str = "files:";
pub const ATTEMPTS_PREFIX: &str = "attempts:";
pub const INVITE_PREFIX: &str = "invite:";
pub const GRADES_PREFIX: &str = "grades:";
pub const CONTACT_PREFIX: &str = "contact:";
pub const MAKEUP_PREFIX: &str = "makeup:";
pub const PUBLISH_PREFIX: &str = "publish:";
pub const ARCHIVE_PREFIX: &str = "archive:";
pub const DEADLINE_PREFIX: &str = "deadline:";
pub const STUDENT_PREFIX: &str = "student:";
pub const MAX_ATTEMPTS_PREFIX: &str = "maxattempts:";
pub const EXTRA_ATTEMPTS_PREFIX: &str = "attempt:";
pub const GRADING_PREFIX: &str = "grading:";
pub const REVIEW_SCORE_PREFIX: &str = "reviewscore:";
pub const LATE_POLICY_PREFIX: &str = "late:";
const STRICT_LATE_POLICY: &str = "none";
pub const PENALTY_PREFIX: &str = "penalty:";
pub const PENALTY_PERIOD: &str = "penalty:period";
pub const TERMS_SAVE: &str = "terms:save";
pub const TERMS_DEADLINE: &str = "terms:deadline";
pub const TERMS_PENALTY: &str = "terms:penalty";
pub const TERMS_PERIOD: &str = "terms:period";
pub const TERMS_PERIODS: &str = "terms:periods";
pub const TERMS_POLICY: &str = "terms:policy";
pub const TERMS_BACK: &str = "terms:back";
pub const TERMS_DURATION_DATE: &str = "terms:duration-date";
pub const TERMS_DURATION_APPLY: &str = "terms:duration-apply";
pub const TERMS_DURATION_PERIODS_PREFIX: &str = "terms:duration:";
pub const TERMS_EXIT: &str = "terms:exit";
pub const TERMS_STAY: &str = "terms:stay";
pub const LATE_PERIODS_PREFIX: &str = "periods:";
pub const TERMS_CANCEL: &str = "terms:cancel";
pub const TERMS_MAKEUP: &str = "terms:makeup";

pub const MAKEUP_ANSWER_PREFIX: &str = "makeup-answer:";
const MANUAL_CHOICE: &str = "manual";
const PASS_FAIL_GRADING: &str = "passfail";
const DATE_FORMAT: &str = "%Y-%m-%d";
const MONTH_FORMAT: &str = "%Y-%m";
const MAKEUP_ANSWER_MANUAL: &str = MANUAL_CHOICE;
const MAKEUP_ANSWER_NOT_OFFERED: &str = "off";
const MAKEUP_ANSWER_PASS_FAIL: &str = "pass";
pub const MAKEUP_ATTEMPTS_PREFIX: &str = "makeup-attempts:";
pub const MAKEUP_DATE: &str = "makeup-form:date";
pub const MAKEUP_ATTEMPTS: &str = "makeup-form:attempts";
pub const MAKEUP_DISABLE: &str = "makeup-form:disable";
pub const MAKEUP_SAVE: &str = "makeup-form:save";
pub const MAKEUP_CANCEL: &str = "makeup-form:cancel";
pub const MAKEUP_BACK: &str = "makeup-form:back";
pub const MAKEUP_GRANT: &str = "makeup-grant";

pub const FILE_ADD: &str = "file:add";
pub const FILE_FINISH: &str = "file:finish";
pub const FILE_CONFIRM_DELETE: &str = "file:confirm-delete";
pub const FILE_CANCEL_DELETE: &str = "file:cancel-delete";
pub const FILE_OPEN_PREFIX: &str = "file:open:";
pub const FILE_DELETE_PREFIX: &str = "file:delete:";

pub const INVITE_ROTATE: &str = "invite:rotate";
pub const INVITE_CONFIRM_ROTATE: &str = "invite:confirm-rotate";
pub const INVITE_CANCEL_ROTATE: &str = "invite:cancel-rotate";
pub const REGISTRATION_GROUP_PREFIX: &str = "reg-group:";
pub const ASSIGNMENT_GROUP_PREFIX: &str = "agroup:";
pub const ASSIGNMENT_GROUPS_DONE: &str = "agroup:done";

pub const REVIEW_ACCEPT_PREFIX: &str = "review:accept:";
pub const REVIEW_REJECT_PREFIX: &str = "review:reject:";
pub const REVIEW_OPEN_PREFIX: &str = "review:open:";
pub const REVIEW_CHANGE_PREFIX: &str = "review:change:";
pub const SANDBOX_ACCEPT_ALL: &str = "sandbox:accept-all";
pub const SANDBOX_REJECT_ALL: &str = "sandbox:reject-all";

pub const DEADLINE_CALENDAR: &str = "dl:calendar";
pub const DEADLINE_MANUAL: &str = "dl:manual";
pub const DEADLINE_OPTIONS: &str = "dl:options";
pub const CALENDAR_PICK_PREFIX: &str = "cal:pick:";
pub const CALENDAR_MONTH_PREFIX: &str = "cal:";
pub const CALENDAR_NOOP: &str = "cal:noop";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CallbackAction {
    Replayable(ReplayableAction),
    Screen(ScreenAction),
    Flow(FlowAction),
    Informational(InformationalAction),
    Noop,
}

impl CallbackAction {
    pub fn callback(self) -> String {
        match self {
            Replayable(action) => action.callback(),
            Screen(action) => action.callback(),
            Flow(action) => action.callback(),
            Informational(action) => action.callback(),
            Self::Noop => CALENDAR_NOOP.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InformationalAction {
    StudentContact {
        course_id: TeachingCourseId,
        student_id: UserId,
    },
}

impl InformationalAction {
    pub fn callback(self) -> String {
        match self {
            Self::StudentContact {
                course_id,
                student_id,
            } => format!(
                "{CONTACT_PREFIX}{}:{}",
                encode_id(course_id.as_uuid()),
                encode_id(student_id.as_uuid()),
            ),
        }
    }
}

fn encode_id(id: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(id.as_bytes())
}

fn decode_id(value: &str) -> Option<Uuid> {
    Uuid::from_slice(&URL_SAFE_NO_PAD.decode(value).ok()?).ok()
}

fn parse_contact(data: &str) -> Option<InformationalAction> {
    if data.len() > 64 {
        return None;
    }
    let (course, student) = data.strip_prefix(CONTACT_PREFIX)?.split_once(':')?;
    Some(InformationalAction::StudentContact {
        course_id: TeachingCourseId::from_uuid(decode_id(course)?),
        student_id: UserId::from_uuid(decode_id(student)?),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplayableAction {
    Page(PageTarget, u32),
    Profile,
    ReviewHub,
    NextReview,
    NextMakeupReview,
    CourseMakeup(TeachingCourseId),
    Assignment(AssignmentId),
    ManageAssignment(AssignmentId),
    StartAssignmentCreation(TeachingCourseId),
    StartSubmission(AssignmentId),
    AssignmentFiles(AssignmentId),
    CourseInvite(TeachingCourseId),
    GradebookGroups(TeachingCourseId),
    ReviewSubmission(SubmissionId),
}

impl ReplayableAction {
    pub fn callback(self) -> String {
        match self {
            Self::Page(PageTarget::Assignments, 0) => MENU_ASSIGNMENTS.to_owned(),
            Self::Page(PageTarget::Archive, 0) => MENU_ARCHIVE.to_owned(),
            Self::Page(PageTarget::Queue, 0) => MENU_QUEUE.to_owned(),
            Self::Page(PageTarget::Courses, 0) => MENU_COURSES.to_owned(),
            Self::Page(PageTarget::Sandbox, 0) => MENU_SANDBOX.to_owned(),
            Self::Page(PageTarget::Course(id), 0) => format!("{COURSE_PREFIX}{}", id.as_uuid()),
            Self::Page(PageTarget::Students(id), 0) => format!("{ATTEMPTS_PREFIX}{}", id.as_uuid()),
            Self::Page(target, page) => target.callback(page),
            Self::Profile => MENU_PROFILE.to_owned(),
            Self::ReviewHub => REVIEW_HUB.to_owned(),
            Self::NextReview => NEXT_REVIEW.to_owned(),
            Self::NextMakeupReview => NEXT_MAKEUP_REVIEW.to_owned(),
            Self::CourseMakeup(id) => format!("{MAKEUP_PREFIX}{}", id.as_uuid()),
            Self::Assignment(id) => format!("{ASSIGNMENT_PREFIX}{}", id.as_uuid()),
            Self::ManageAssignment(id) => format!("{MANAGE_PREFIX}{}", id.as_uuid()),
            Self::StartAssignmentCreation(id) => format!("{CREATE_PREFIX}{}", id.as_uuid()),
            Self::StartSubmission(id) => format!("{SUBMIT_PREFIX}{}", id.as_uuid()),
            Self::AssignmentFiles(id) => format!("{FILES_PREFIX}{}", id.as_uuid()),
            Self::CourseInvite(id) => format!("{INVITE_PREFIX}{}", id.as_uuid()),
            Self::GradebookGroups(id) => format!("{GRADES_PREFIX}{}", id.as_uuid()),
            Self::ReviewSubmission(id) => format!("{REVIEW_OPEN_PREFIX}{}", id.as_uuid()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PageTarget {
    Assignments,
    Archive,
    Queue,
    Courses,
    Sandbox,
    Course(TeachingCourseId),
    Students(AssignmentId),
    Gradebook(TeachingCourseId, StudentGroupId),
    History(AssignmentId),
    StudentGrades(TeachingCourseId, UserId),
    Roster(TeachingCourseId),
}

impl PageTarget {
    pub fn callback(self, page: u32) -> String {
        let id = encode_id;
        let target = match self {
            Self::Assignments => "a".to_owned(),
            Self::Archive => "ar".to_owned(),
            Self::Queue => "q".to_owned(),
            Self::Courses => "c".to_owned(),
            Self::Sandbox => "sb".to_owned(),
            Self::Course(course) => format!("ca:{}", id(course.as_uuid())),
            Self::Students(assignment) => format!("s:{}", id(assignment.as_uuid())),
            Self::Gradebook(course, group) => {
                format!("g:{}:{}", id(course.as_uuid()), id(group.as_uuid()))
            }
            Self::History(assignment) => format!("h:{}", id(assignment.as_uuid())),
            Self::StudentGrades(course, student) => {
                format!("sg:{}:{}", id(course.as_uuid()), id(student.as_uuid()))
            }
            Self::Roster(course) => format!("r:{}", id(course.as_uuid())),
        };
        format!("{PAGE_PREFIX}{target}:{page}")
    }
}

fn parse_page(data: &str) -> Option<ReplayableAction> {
    let value = data.strip_prefix(PAGE_PREFIX)?;
    let (target, page) = value.rsplit_once(':')?;
    if page.is_empty() || !page.bytes().all(|byte| byte.is_ascii_digit()) || data.len() > 64 {
        return None;
    }
    let page = page.parse().ok()?;
    let mut parts = target.split(':');
    let kind = parts.next()?;
    let mut id = || decode_id(parts.next()?);
    let target = match kind {
        "a" => PageTarget::Assignments,
        "ar" => PageTarget::Archive,
        "q" => PageTarget::Queue,
        "c" => PageTarget::Courses,
        "sb" => PageTarget::Sandbox,
        "ca" => PageTarget::Course(TeachingCourseId::from_uuid(id()?)),
        "s" => PageTarget::Students(AssignmentId::from_uuid(id()?)),
        "g" => PageTarget::Gradebook(
            TeachingCourseId::from_uuid(id()?),
            StudentGroupId::from_uuid(id()?),
        ),
        "h" => PageTarget::History(AssignmentId::from_uuid(id()?)),
        "sg" => {
            PageTarget::StudentGrades(TeachingCourseId::from_uuid(id()?), UserId::from_uuid(id()?))
        }
        "r" => PageTarget::Roster(TeachingCourseId::from_uuid(id()?)),
        _ => return None,
    };
    parts
        .next()
        .is_none()
        .then_some(ReplayableAction::Page(target, page))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScreenAction {
    Publish(AssignmentId),
    Archive(AssignmentId),
    ChangeDeadline(AssignmentId),
    AddFile,
    OpenFile(usize),
    DeleteFile(usize),
    FinishFileCreation,
    RotateCourseInvite,
    BulkReview(ReviewDecision),
    GrantMakeupAttempt,
}

impl ScreenAction {
    pub fn callback(self) -> String {
        match self {
            Self::Publish(id) => format!("{PUBLISH_PREFIX}{}", id.as_uuid()),
            Self::Archive(id) => format!("{ARCHIVE_PREFIX}{}", id.as_uuid()),
            Self::ChangeDeadline(id) => format!("{DEADLINE_PREFIX}{}", id.as_uuid()),
            Self::AddFile => FILE_ADD.to_owned(),
            Self::OpenFile(index) => format!("{FILE_OPEN_PREFIX}{index}"),
            Self::DeleteFile(index) => format!("{FILE_DELETE_PREFIX}{index}"),
            Self::FinishFileCreation => FILE_FINISH.to_owned(),
            Self::RotateCourseInvite => INVITE_ROTATE.to_owned(),
            Self::BulkReview(ReviewDecision::Accept) => SANDBOX_ACCEPT_ALL.to_owned(),
            Self::BulkReview(ReviewDecision::Reject) => SANDBOX_REJECT_ALL.to_owned(),
            Self::GrantMakeupAttempt => MAKEUP_GRANT.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FlowAction {
    Review {
        decision: ReviewDecision,
        submission_id: SubmissionId,
    },
    ChangeReviewDecision(SubmissionId),
    MaxAttempts(AmountChoice),
    Grading(GradingChoice),
    LatePolicy(LatePolicyChoice),
    Penalty(AmountChoice),
    Periods(AmountChoice),
    DurationPeriods(u16),
    PenaltyPeriod,
    Terms(TermsAction),
    ReviewScore {
        submission_id: SubmissionId,
        choice: AmountChoice,
    },
    Deadline(DeadlineAction),
    SelectStudent(UserId),
    ExtraAttempts(AmountChoice),
    ConfirmFileDeletion,
    CancelFileDeletion,
    ConfirmInviteRotation,
    CancelInviteRotation,
    RegistrationGroup(StudentGroupId),
    AssignmentGroup(StudentGroupId),
    AssignmentGroupsDone,
    MakeupAnswer(MakeupChoice),
    MakeupAttempts(AmountChoice),
    MakeupForm(MakeupFormAction),
}

impl FlowAction {
    pub fn callback(self) -> String {
        match self {
            Self::Review {
                decision: ReviewDecision::Accept,
                submission_id,
            } => format!("{REVIEW_ACCEPT_PREFIX}{}", submission_id.as_uuid()),
            Self::Review {
                decision: ReviewDecision::Reject,
                submission_id,
            } => format!("{REVIEW_REJECT_PREFIX}{}", submission_id.as_uuid()),
            Self::ChangeReviewDecision(id) => format!("{REVIEW_CHANGE_PREFIX}{}", id.as_uuid()),
            Self::MaxAttempts(choice) => choice.callback(MAX_ATTEMPTS_PREFIX),
            Self::Grading(choice) => choice.callback(),
            Self::LatePolicy(choice) => choice.callback(),
            Self::Penalty(choice) => choice.callback(PENALTY_PREFIX),
            Self::Periods(choice) => choice.callback(LATE_PERIODS_PREFIX),
            Self::DurationPeriods(periods) => format!("{TERMS_DURATION_PERIODS_PREFIX}{periods}"),
            Self::PenaltyPeriod => PENALTY_PERIOD.to_owned(),
            Self::Terms(action) => action.callback().to_owned(),
            Self::ReviewScore {
                submission_id,
                choice,
            } => choice.callback(&format!(
                "{REVIEW_SCORE_PREFIX}{}:",
                submission_id.as_uuid()
            )),
            Self::Deadline(action) => action.callback(),
            Self::SelectStudent(id) => format!("{STUDENT_PREFIX}{}", id.as_uuid()),
            Self::ExtraAttempts(choice) => choice.callback(EXTRA_ATTEMPTS_PREFIX),
            Self::ConfirmFileDeletion => FILE_CONFIRM_DELETE.to_owned(),
            Self::CancelFileDeletion => FILE_CANCEL_DELETE.to_owned(),
            Self::ConfirmInviteRotation => INVITE_CONFIRM_ROTATE.to_owned(),
            Self::CancelInviteRotation => INVITE_CANCEL_ROTATE.to_owned(),
            Self::RegistrationGroup(id) => format!("{REGISTRATION_GROUP_PREFIX}{}", id.as_uuid()),
            Self::AssignmentGroup(id) => format!("{ASSIGNMENT_GROUP_PREFIX}{}", id.as_uuid()),
            Self::AssignmentGroupsDone => ASSIGNMENT_GROUPS_DONE.to_owned(),
            Self::MakeupAnswer(choice) => choice.callback(),
            Self::MakeupAttempts(choice) => choice.callback(MAKEUP_ATTEMPTS_PREFIX),
            Self::MakeupForm(action) => action.callback().to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MakeupChoice {
    Credit(u16),
    Manual,
    NotOffered,
    PassFail,
}

impl MakeupChoice {
    fn callback(self) -> String {
        let value = match self {
            Self::Credit(credit) => credit.to_string(),
            Self::Manual => MAKEUP_ANSWER_MANUAL.to_owned(),
            Self::NotOffered => MAKEUP_ANSWER_NOT_OFFERED.to_owned(),
            Self::PassFail => MAKEUP_ANSWER_PASS_FAIL.to_owned(),
        };
        format!("{MAKEUP_ANSWER_PREFIX}{value}")
    }

    pub(super) fn terms(self, grading: Grading) -> Option<DomainResult<MakeupTerms>> {
        Some(match self {
            Self::Manual => return None,
            Self::NotOffered => Ok(MakeupTerms::NotOffered),
            Self::PassFail => Ok(MakeupTerms::PassFail),
            Self::Credit(credit) => MakeupTerms::scored(credit, grading),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MakeupFormAction {
    Date,
    Attempts,
    Disable,
    Save,
    Cancel,
    Back,
}

impl MakeupFormAction {
    fn callback(self) -> &'static str {
        match self {
            Self::Date => MAKEUP_DATE,
            Self::Attempts => MAKEUP_ATTEMPTS,
            Self::Disable => MAKEUP_DISABLE,
            Self::Save => MAKEUP_SAVE,
            Self::Cancel => MAKEUP_CANCEL,
            Self::Back => MAKEUP_BACK,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AmountChoice {
    Preset(u16),
    Manual,
}

impl AmountChoice {
    fn callback(self, prefix: &str) -> String {
        match self {
            Self::Preset(amount) => format!("{prefix}{amount}"),
            Self::Manual => format!("{prefix}{MANUAL_CHOICE}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GradingChoice {
    PassFail,
    Preset(u16),
    Manual,
}

impl GradingChoice {
    fn callback(self) -> String {
        match self {
            Self::PassFail => format!("{GRADING_PREFIX}{PASS_FAIL_GRADING}"),
            Self::Preset(max_score) => format!("{GRADING_PREFIX}{max_score}"),
            Self::Manual => format!("{GRADING_PREFIX}{MANUAL_CHOICE}"),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LatePolicyChoice {
    Strict,
    Penalty(DeductedFrom),
}

impl LatePolicyChoice {
    fn callback(self) -> String {
        let value = match self {
            Self::Strict => STRICT_LATE_POLICY,
            Self::Penalty(deducted_from) => deducted_from.as_str(),
        };
        format!("{LATE_POLICY_PREFIX}{value}")
    }

    pub fn deducted_from(self) -> Option<DeductedFrom> {
        match self {
            Self::Strict => None,
            Self::Penalty(deducted_from) => Some(deducted_from),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TermsAction {
    Save,
    Deadline,
    Penalty,
    Period,
    Periods,
    Policy,
    Back,
    DurationDate,
    DurationApply,
    Exit,
    Stay,
    Cancel,
    Makeup,
}

impl TermsAction {
    fn callback(self) -> &'static str {
        match self {
            Self::Save => TERMS_SAVE,
            Self::Deadline => TERMS_DEADLINE,
            Self::Penalty => TERMS_PENALTY,
            Self::Period => TERMS_PERIOD,
            Self::Periods => TERMS_PERIODS,
            Self::Policy => TERMS_POLICY,
            Self::Back => TERMS_BACK,
            Self::DurationDate => TERMS_DURATION_DATE,
            Self::DurationApply => TERMS_DURATION_APPLY,
            Self::Exit => TERMS_EXIT,
            Self::Stay => TERMS_STAY,
            Self::Cancel => TERMS_CANCEL,
            Self::Makeup => TERMS_MAKEUP,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeadlineAction {
    Calendar,
    Manual,
    Options,
    Month(NaiveDate),
    Pick(NaiveDate),
}

impl DeadlineAction {
    fn callback(self) -> String {
        match self {
            Self::Calendar => DEADLINE_CALENDAR.to_owned(),
            Self::Manual => DEADLINE_MANUAL.to_owned(),
            Self::Options => DEADLINE_OPTIONS.to_owned(),
            Self::Month(month) => {
                format!("{CALENDAR_MONTH_PREFIX}{}", month.format(MONTH_FORMAT))
            }
            Self::Pick(date) => format!("{CALENDAR_PICK_PREFIX}{}", date.format(DATE_FORMAT)),
        }
    }
}

pub(super) fn parse(data: &str) -> Option<CallbackAction> {
    let literal = match data {
        MENU_ASSIGNMENTS => Replayable(ReplayableAction::Page(PageTarget::Assignments, 0)),
        MENU_ARCHIVE => Replayable(ReplayableAction::Page(PageTarget::Archive, 0)),
        MENU_QUEUE => Replayable(ReplayableAction::Page(PageTarget::Queue, 0)),
        MENU_PROFILE => Replayable(ReplayableAction::Profile),
        MENU_COURSES => Replayable(ReplayableAction::Page(PageTarget::Courses, 0)),
        MENU_SANDBOX => Replayable(ReplayableAction::Page(PageTarget::Sandbox, 0)),
        REVIEW_HUB => Replayable(ReplayableAction::ReviewHub),
        NEXT_REVIEW => Replayable(ReplayableAction::NextReview),
        NEXT_MAKEUP_REVIEW => Replayable(ReplayableAction::NextMakeupReview),
        FILE_ADD => Screen(ScreenAction::AddFile),
        FILE_FINISH => Screen(ScreenAction::FinishFileCreation),
        INVITE_ROTATE => Screen(ScreenAction::RotateCourseInvite),
        SANDBOX_ACCEPT_ALL => Screen(ScreenAction::BulkReview(ReviewDecision::Accept)),
        SANDBOX_REJECT_ALL => Screen(ScreenAction::BulkReview(ReviewDecision::Reject)),
        MAKEUP_GRANT => Screen(ScreenAction::GrantMakeupAttempt),
        FILE_CONFIRM_DELETE => Flow(FlowAction::ConfirmFileDeletion),
        FILE_CANCEL_DELETE => Flow(FlowAction::CancelFileDeletion),
        INVITE_CONFIRM_ROTATE => Flow(FlowAction::ConfirmInviteRotation),
        INVITE_CANCEL_ROTATE => Flow(FlowAction::CancelInviteRotation),
        ASSIGNMENT_GROUPS_DONE => Flow(FlowAction::AssignmentGroupsDone),
        PENALTY_PERIOD => Flow(FlowAction::PenaltyPeriod),
        TERMS_SAVE => Flow(FlowAction::Terms(TermsAction::Save)),
        TERMS_DEADLINE => Flow(FlowAction::Terms(TermsAction::Deadline)),
        TERMS_PENALTY => Flow(FlowAction::Terms(TermsAction::Penalty)),
        TERMS_PERIOD => Flow(FlowAction::Terms(TermsAction::Period)),
        TERMS_PERIODS => Flow(FlowAction::Terms(TermsAction::Periods)),
        TERMS_POLICY => Flow(FlowAction::Terms(TermsAction::Policy)),
        TERMS_BACK => Flow(FlowAction::Terms(TermsAction::Back)),
        TERMS_DURATION_DATE => Flow(FlowAction::Terms(TermsAction::DurationDate)),
        TERMS_DURATION_APPLY => Flow(FlowAction::Terms(TermsAction::DurationApply)),
        TERMS_EXIT => Flow(FlowAction::Terms(TermsAction::Exit)),
        TERMS_STAY => Flow(FlowAction::Terms(TermsAction::Stay)),
        TERMS_CANCEL => Flow(FlowAction::Terms(TermsAction::Cancel)),
        TERMS_MAKEUP => Flow(FlowAction::Terms(TermsAction::Makeup)),
        MAKEUP_DATE => Flow(FlowAction::MakeupForm(MakeupFormAction::Date)),
        MAKEUP_ATTEMPTS => Flow(FlowAction::MakeupForm(MakeupFormAction::Attempts)),
        MAKEUP_DISABLE => Flow(FlowAction::MakeupForm(MakeupFormAction::Disable)),
        MAKEUP_SAVE => Flow(FlowAction::MakeupForm(MakeupFormAction::Save)),
        MAKEUP_CANCEL => Flow(FlowAction::MakeupForm(MakeupFormAction::Cancel)),
        MAKEUP_BACK => Flow(FlowAction::MakeupForm(MakeupFormAction::Back)),
        DEADLINE_CALENDAR => Flow(FlowAction::Deadline(DeadlineAction::Calendar)),
        DEADLINE_MANUAL => Flow(FlowAction::Deadline(DeadlineAction::Manual)),
        DEADLINE_OPTIONS => Flow(FlowAction::Deadline(DeadlineAction::Options)),
        CALENDAR_NOOP => CallbackAction::Noop,
        _ => return parse_prefixed(data),
    };
    Some(literal)
}

type Route<V, A> = (&'static str, fn(V) -> A);

const ID_ROUTES: &[Route<Uuid, CallbackAction>] = &[
    (COURSE_PREFIX, |id| {
        Replayable(ReplayableAction::Page(
            PageTarget::Course(TeachingCourseId::from_uuid(id)),
            0,
        ))
    }),
    (MAKEUP_PREFIX, |id| {
        Replayable(ReplayableAction::CourseMakeup(TeachingCourseId::from_uuid(
            id,
        )))
    }),
    (ASSIGNMENT_PREFIX, |id| {
        Replayable(ReplayableAction::Assignment(AssignmentId::from_uuid(id)))
    }),
    (MANAGE_PREFIX, |id| {
        Replayable(ReplayableAction::ManageAssignment(AssignmentId::from_uuid(
            id,
        )))
    }),
    (CREATE_PREFIX, |id| {
        Replayable(ReplayableAction::StartAssignmentCreation(
            TeachingCourseId::from_uuid(id),
        ))
    }),
    (SUBMIT_PREFIX, |id| {
        Replayable(ReplayableAction::StartSubmission(AssignmentId::from_uuid(
            id,
        )))
    }),
    (FILES_PREFIX, |id| {
        Replayable(ReplayableAction::AssignmentFiles(AssignmentId::from_uuid(
            id,
        )))
    }),
    (ATTEMPTS_PREFIX, |id| {
        Replayable(ReplayableAction::Page(
            PageTarget::Students(AssignmentId::from_uuid(id)),
            0,
        ))
    }),
    (INVITE_PREFIX, |id| {
        Replayable(ReplayableAction::CourseInvite(TeachingCourseId::from_uuid(
            id,
        )))
    }),
    (GRADES_PREFIX, |id| {
        Replayable(ReplayableAction::GradebookGroups(
            TeachingCourseId::from_uuid(id),
        ))
    }),
    (REVIEW_OPEN_PREFIX, |id| {
        Replayable(ReplayableAction::ReviewSubmission(SubmissionId::from_uuid(
            id,
        )))
    }),
    (PUBLISH_PREFIX, |id| {
        Screen(ScreenAction::Publish(AssignmentId::from_uuid(id)))
    }),
    (ARCHIVE_PREFIX, |id| {
        Screen(ScreenAction::Archive(AssignmentId::from_uuid(id)))
    }),
    (DEADLINE_PREFIX, |id| {
        Screen(ScreenAction::ChangeDeadline(AssignmentId::from_uuid(id)))
    }),
    (REVIEW_ACCEPT_PREFIX, |id| {
        Flow(FlowAction::Review {
            decision: ReviewDecision::Accept,
            submission_id: SubmissionId::from_uuid(id),
        })
    }),
    (REVIEW_REJECT_PREFIX, |id| {
        Flow(FlowAction::Review {
            decision: ReviewDecision::Reject,
            submission_id: SubmissionId::from_uuid(id),
        })
    }),
    (REVIEW_CHANGE_PREFIX, |id| {
        Flow(FlowAction::ChangeReviewDecision(SubmissionId::from_uuid(
            id,
        )))
    }),
    (STUDENT_PREFIX, |id| {
        Flow(FlowAction::SelectStudent(UserId::from_uuid(id)))
    }),
    (REGISTRATION_GROUP_PREFIX, |id| {
        Flow(FlowAction::RegistrationGroup(StudentGroupId::from_uuid(id)))
    }),
    (ASSIGNMENT_GROUP_PREFIX, |id| {
        Flow(FlowAction::AssignmentGroup(StudentGroupId::from_uuid(id)))
    }),
];

const AMOUNT_ROUTES: &[Route<AmountChoice, FlowAction>] = &[
    (MAX_ATTEMPTS_PREFIX, FlowAction::MaxAttempts),
    (EXTRA_ATTEMPTS_PREFIX, FlowAction::ExtraAttempts),
    (PENALTY_PREFIX, FlowAction::Penalty),
    (LATE_PERIODS_PREFIX, FlowAction::Periods),
    (MAKEUP_ATTEMPTS_PREFIX, FlowAction::MakeupAttempts),
];

const INDEX_ROUTES: &[Route<usize, ScreenAction>] = &[
    (FILE_OPEN_PREFIX, ScreenAction::OpenFile),
    (FILE_DELETE_PREFIX, ScreenAction::DeleteFile),
];

fn parse_prefixed(data: &str) -> Option<CallbackAction> {
    if data.starts_with(CONTACT_PREFIX) {
        return parse_contact(data).map(Informational);
    }
    if data.starts_with(PAGE_PREFIX) {
        return parse_page(data).map(Replayable);
    }
    route(ID_ROUTES, data, |value| value.parse().ok())
        .or_else(|| route(AMOUNT_ROUTES, data, parse_amount_value).map(Flow))
        .or_else(|| route(INDEX_ROUTES, data, |value| value.parse().ok()).map(Screen))
        .or_else(|| parse_flow(data).map(Flow))
}

fn route<V, A>(routes: &[Route<V, A>], data: &str, parse: impl Fn(&str) -> Option<V>) -> Option<A> {
    routes.iter().find_map(|(prefix, build)| {
        let value = parse(data.strip_prefix(prefix)?)?;
        Some(build(value))
    })
}

fn parse_flow(data: &str) -> Option<FlowAction> {
    if let Some(choice) = data.strip_prefix(GRADING_PREFIX) {
        parse_grading(choice).map(FlowAction::Grading)
    } else if let Some(choice) = data.strip_prefix(MAKEUP_ANSWER_PREFIX) {
        parse_makeup_answer(choice).map(FlowAction::MakeupAnswer)
    } else if let Some(choice) = data.strip_prefix(LATE_POLICY_PREFIX) {
        parse_late_policy(choice).map(FlowAction::LatePolicy)
    } else if let Some(score) = data.strip_prefix(REVIEW_SCORE_PREFIX) {
        parse_review_score(score)
    } else if let Some(periods) = data.strip_prefix(TERMS_DURATION_PERIODS_PREFIX) {
        parse_u16(periods).map(FlowAction::DurationPeriods)
    } else {
        parse_deadline(data).map(FlowAction::Deadline)
    }
}

fn parse_review_score(value: &str) -> Option<FlowAction> {
    let (submission_id, choice) = value.split_once(':')?;
    let submission_id = SubmissionId::from_uuid(submission_id.parse().ok()?);
    let choice = parse_amount_value(choice)?;

    Some(FlowAction::ReviewScore {
        submission_id,
        choice,
    })
}

fn parse_grading(choice: &str) -> Option<GradingChoice> {
    match choice {
        PASS_FAIL_GRADING => Some(GradingChoice::PassFail),
        MANUAL_CHOICE => Some(GradingChoice::Manual),
        value => parse_u16(value).map(GradingChoice::Preset),
    }
}

fn parse_makeup_answer(choice: &str) -> Option<MakeupChoice> {
    match choice {
        MAKEUP_ANSWER_MANUAL => Some(MakeupChoice::Manual),
        MAKEUP_ANSWER_NOT_OFFERED => Some(MakeupChoice::NotOffered),
        MAKEUP_ANSWER_PASS_FAIL => Some(MakeupChoice::PassFail),
        value => parse_u16(value).map(MakeupChoice::Credit),
    }
}

fn parse_late_policy(choice: &str) -> Option<LatePolicyChoice> {
    match choice {
        STRICT_LATE_POLICY => Some(LatePolicyChoice::Strict),
        value => DeductedFrom::parse_str(value).map(LatePolicyChoice::Penalty),
    }
}

fn parse_amount_value(value: &str) -> Option<AmountChoice> {
    if value == MANUAL_CHOICE {
        return Some(AmountChoice::Manual);
    }
    parse_u16(value).map(AmountChoice::Preset)
}

fn parse_deadline(data: &str) -> Option<DeadlineAction> {
    // cal: поглотил бы cal:pick:, если разобрать общий префикс первым.
    if let Some(date) = data.strip_prefix(CALENDAR_PICK_PREFIX) {
        return NaiveDate::parse_from_str(date, DATE_FORMAT)
            .ok()
            .map(DeadlineAction::Pick);
    }
    data.strip_prefix(CALENDAR_MONTH_PREFIX)
        .and_then(parse_month)
        .map(DeadlineAction::Month)
}

fn parse_month(value: &str) -> Option<NaiveDate> {
    let (year, month) = value.split_once('-')?;
    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, 1)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn student_contact_is_informational_and_fits_two_ids_in_53_bytes() {
        let target = InformationalAction::StudentContact {
            course_id: TeachingCourseId::new(),
            student_id: UserId::new(),
        };
        let data = target.callback();
        assert_eq!(data.len(), 53);
        assert_eq!(parse(&data), Some(Informational(target)));
    }

    #[test]
    fn malformed_contact_targets_are_rejected() {
        let id = encode_id(Uuid::new_v4());
        for invalid in [
            format!("{CONTACT_PREFIX}{id}"),
            format!("{CONTACT_PREFIX}{id}:"),
            format!("{CONTACT_PREFIX}:{id}"),
            format!("{CONTACT_PREFIX}{id}:{id}:extra"),
            format!("{CONTACT_PREFIX}{id}:{id}=="),
            format!("{CONTACT_PREFIX}!invalid!:{id}"),
            format!("{CONTACT_PREFIX}{}:{id}", URL_SAFE_NO_PAD.encode([1; 15])),
            format!("{CONTACT_PREFIX}{id}:{}", URL_SAFE_NO_PAD.encode([1; 17])),
            format!("{CONTACT_PREFIX}{id}:{}", "A".repeat(65)),
        ] {
            assert_eq!(parse(&invalid), None, "{invalid}");
        }
    }

    #[test]
    fn a_first_page_round_trips_through_its_short_route() {
        for target in [
            PageTarget::Assignments,
            PageTarget::Archive,
            PageTarget::Queue,
            PageTarget::Courses,
            PageTarget::Sandbox,
            PageTarget::Course(TeachingCourseId::new()),
            PageTarget::Students(AssignmentId::new()),
        ] {
            let data = ReplayableAction::Page(target, 0).callback();
            assert!(!data.starts_with(PAGE_PREFIX), "{data}");
            assert_eq!(
                parse(&data),
                Some(CallbackAction::Replayable(ReplayableAction::Page(
                    target, 0
                )))
            );
        }
    }

    #[test]
    fn pagination_routes_fit_telegram_from_the_first_to_the_last_page() {
        for target in [
            PageTarget::Assignments,
            PageTarget::Archive,
            PageTarget::Queue,
            PageTarget::Courses,
            PageTarget::Sandbox,
            PageTarget::Course(TeachingCourseId::new()),
            PageTarget::Students(AssignmentId::new()),
            PageTarget::Gradebook(TeachingCourseId::new(), StudentGroupId::new()),
            PageTarget::History(AssignmentId::new()),
            PageTarget::StudentGrades(TeachingCourseId::new(), UserId::new()),
            PageTarget::Roster(TeachingCourseId::new()),
        ] {
            for page in [0, u32::MAX] {
                let data = target.callback(page);
                assert!(data.len() <= 64, "{data}");
                assert_eq!(
                    parse(&data),
                    Some(Replayable(ReplayableAction::Page(target, page))),
                    "{data}"
                );
            }
        }
        for invalid in [
            "pg:a:-1",
            "pg:a:4294967296",
            "pg:a:extra:0",
            "pg:sg:bad:bad:0",
            "pg:g:bad:0",
            "pg:r:bad:0",
            "pg:r:0",
            "pg:a:",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }

    macro_rules! one_of_each {
        ($($variant:pat => $sample:expr),+ $(,)?) => {{
            fn _every_variant_has_a_sample(action: CallbackAction) {
                match action {
                    $($variant => {})+
                }
            }
            vec![$({
                let sample: CallbackAction = $sample;
                assert!(matches!(sample, $variant), "{sample:?}");
                sample
            }),+]
        }};
    }

    fn one_sample_of_each_action() -> Vec<CallbackAction> {
        let course =
            TeachingCourseId::from_uuid(Uuid::from_u128(0x0192_6f3a_7b1c_7d2e_8f40_51a2_b3c4_d5e6));
        let assignment =
            AssignmentId::from_uuid(Uuid::from_u128(0x0192_6f3a_7b1c_7d2e_9a41_62b3_c4d5_e6f7));
        let submission =
            SubmissionId::from_uuid(Uuid::from_u128(0x0192_6f3a_7b1c_7d2e_ab52_73c4_d5e6_f708));
        let student = UserId::from_uuid(Uuid::from_u128(0x0192_6f3a_7b1c_7d2e_bc63_84d5_e6f7_0819));
        let group =
            StudentGroupId::from_uuid(Uuid::from_u128(0x3f2a_1b0c_9d8e_4f70_a162_5344_2635_1708));
        let day = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let month = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let last = u32::MAX;
        let most = u16::MAX;
        let page = |target, page| Replayable(ReplayableAction::Page(target, page));
        let review = |decision| {
            Flow(FlowAction::Review {
                decision,
                submission_id: submission,
            })
        };
        let score = |choice| {
            Flow(FlowAction::ReviewScore {
                submission_id: submission,
                choice,
            })
        };

        one_of_each![
            Replayable(ReplayableAction::Page(PageTarget::Assignments, 0)) =>
                page(PageTarget::Assignments, 0),
            Replayable(ReplayableAction::Page(PageTarget::Assignments, 1..=u32::MAX)) =>
                page(PageTarget::Assignments, last),
            Replayable(ReplayableAction::Page(PageTarget::Archive, 0)) =>
                page(PageTarget::Archive, 0),
            Replayable(ReplayableAction::Page(PageTarget::Archive, 1..=u32::MAX)) =>
                page(PageTarget::Archive, last),
            Replayable(ReplayableAction::Page(PageTarget::Queue, 0)) =>
                page(PageTarget::Queue, 0),
            Replayable(ReplayableAction::Page(PageTarget::Queue, 1..=u32::MAX)) =>
                page(PageTarget::Queue, last),
            Replayable(ReplayableAction::Page(PageTarget::Courses, 0)) =>
                page(PageTarget::Courses, 0),
            Replayable(ReplayableAction::Page(PageTarget::Courses, 1..=u32::MAX)) =>
                page(PageTarget::Courses, last),
            Replayable(ReplayableAction::Page(PageTarget::Sandbox, 0)) =>
                page(PageTarget::Sandbox, 0),
            Replayable(ReplayableAction::Page(PageTarget::Sandbox, 1..=u32::MAX)) =>
                page(PageTarget::Sandbox, last),
            Replayable(ReplayableAction::Page(PageTarget::Course(_), 0)) =>
                page(PageTarget::Course(course), 0),
            Replayable(ReplayableAction::Page(PageTarget::Course(_), 1..=u32::MAX)) =>
                page(PageTarget::Course(course), last),
            Replayable(ReplayableAction::Page(PageTarget::Students(_), 0)) =>
                page(PageTarget::Students(assignment), 0),
            Replayable(ReplayableAction::Page(PageTarget::Students(_), 1..=u32::MAX)) =>
                page(PageTarget::Students(assignment), last),
            Replayable(ReplayableAction::Page(PageTarget::Gradebook(..), _)) =>
                page(PageTarget::Gradebook(course, group), last),
            Replayable(ReplayableAction::Page(PageTarget::History(_), _)) =>
                page(PageTarget::History(assignment), last),
            Replayable(ReplayableAction::Page(PageTarget::StudentGrades(..), _)) =>
                page(PageTarget::StudentGrades(course, student), last),
            Replayable(ReplayableAction::Page(PageTarget::Roster(_), _)) =>
                page(PageTarget::Roster(course), last),
            Replayable(ReplayableAction::Profile) => Replayable(ReplayableAction::Profile),
            Replayable(ReplayableAction::ReviewHub) => Replayable(ReplayableAction::ReviewHub),
            Replayable(ReplayableAction::NextReview) => Replayable(ReplayableAction::NextReview),
            Replayable(ReplayableAction::NextMakeupReview) =>
                Replayable(ReplayableAction::NextMakeupReview),
            Replayable(ReplayableAction::CourseMakeup(_)) =>
                Replayable(ReplayableAction::CourseMakeup(course)),
            Replayable(ReplayableAction::Assignment(_)) =>
                Replayable(ReplayableAction::Assignment(assignment)),
            Replayable(ReplayableAction::ManageAssignment(_)) =>
                Replayable(ReplayableAction::ManageAssignment(assignment)),
            Replayable(ReplayableAction::StartAssignmentCreation(_)) =>
                Replayable(ReplayableAction::StartAssignmentCreation(course)),
            Replayable(ReplayableAction::StartSubmission(_)) =>
                Replayable(ReplayableAction::StartSubmission(assignment)),
            Replayable(ReplayableAction::AssignmentFiles(_)) =>
                Replayable(ReplayableAction::AssignmentFiles(assignment)),
            Replayable(ReplayableAction::CourseInvite(_)) =>
                Replayable(ReplayableAction::CourseInvite(course)),
            Replayable(ReplayableAction::GradebookGroups(_)) =>
                Replayable(ReplayableAction::GradebookGroups(course)),
            Replayable(ReplayableAction::ReviewSubmission(_)) =>
                Replayable(ReplayableAction::ReviewSubmission(submission)),
            Screen(ScreenAction::Publish(_)) => Screen(ScreenAction::Publish(assignment)),
            Screen(ScreenAction::Archive(_)) => Screen(ScreenAction::Archive(assignment)),
            Screen(ScreenAction::ChangeDeadline(_)) =>
                Screen(ScreenAction::ChangeDeadline(assignment)),
            Screen(ScreenAction::AddFile) => Screen(ScreenAction::AddFile),
            Screen(ScreenAction::OpenFile(_)) => Screen(ScreenAction::OpenFile(usize::MAX)),
            Screen(ScreenAction::DeleteFile(_)) => Screen(ScreenAction::DeleteFile(usize::MAX)),
            Screen(ScreenAction::FinishFileCreation) => Screen(ScreenAction::FinishFileCreation),
            Screen(ScreenAction::RotateCourseInvite) => Screen(ScreenAction::RotateCourseInvite),
            Screen(ScreenAction::BulkReview(ReviewDecision::Accept)) =>
                Screen(ScreenAction::BulkReview(ReviewDecision::Accept)),
            Screen(ScreenAction::BulkReview(ReviewDecision::Reject)) =>
                Screen(ScreenAction::BulkReview(ReviewDecision::Reject)),
            Screen(ScreenAction::GrantMakeupAttempt) => Screen(ScreenAction::GrantMakeupAttempt),
            Flow(FlowAction::Review { decision: ReviewDecision::Accept, .. }) =>
                review(ReviewDecision::Accept),
            Flow(FlowAction::Review { decision: ReviewDecision::Reject, .. }) =>
                review(ReviewDecision::Reject),
            Flow(FlowAction::ChangeReviewDecision(_)) =>
                Flow(FlowAction::ChangeReviewDecision(submission)),
            Flow(FlowAction::MaxAttempts(AmountChoice::Preset(_))) =>
                Flow(FlowAction::MaxAttempts(AmountChoice::Preset(most))),
            Flow(FlowAction::MaxAttempts(AmountChoice::Manual)) =>
                Flow(FlowAction::MaxAttempts(AmountChoice::Manual)),
            Flow(FlowAction::Grading(GradingChoice::PassFail)) =>
                Flow(FlowAction::Grading(GradingChoice::PassFail)),
            Flow(FlowAction::Grading(GradingChoice::Preset(_))) =>
                Flow(FlowAction::Grading(GradingChoice::Preset(most))),
            Flow(FlowAction::Grading(GradingChoice::Manual)) =>
                Flow(FlowAction::Grading(GradingChoice::Manual)),
            Flow(FlowAction::LatePolicy(LatePolicyChoice::Strict)) =>
                Flow(FlowAction::LatePolicy(LatePolicyChoice::Strict)),
            Flow(FlowAction::LatePolicy(LatePolicyChoice::Penalty(DeductedFrom::WorkScore))) =>
                Flow(FlowAction::LatePolicy(LatePolicyChoice::Penalty(DeductedFrom::WorkScore))),
            Flow(FlowAction::LatePolicy(LatePolicyChoice::Penalty(DeductedFrom::CourseTotal))) =>
                Flow(FlowAction::LatePolicy(LatePolicyChoice::Penalty(DeductedFrom::CourseTotal))),
            Flow(FlowAction::Penalty(AmountChoice::Preset(_))) =>
                Flow(FlowAction::Penalty(AmountChoice::Preset(most))),
            Flow(FlowAction::Penalty(AmountChoice::Manual)) =>
                Flow(FlowAction::Penalty(AmountChoice::Manual)),
            Flow(FlowAction::Periods(AmountChoice::Preset(_))) =>
                Flow(FlowAction::Periods(AmountChoice::Preset(most))),
            Flow(FlowAction::Periods(AmountChoice::Manual)) =>
                Flow(FlowAction::Periods(AmountChoice::Manual)),
            Flow(FlowAction::DurationPeriods(_)) => Flow(FlowAction::DurationPeriods(most)),
            Flow(FlowAction::PenaltyPeriod) => Flow(FlowAction::PenaltyPeriod),
            Flow(FlowAction::Terms(TermsAction::Save)) =>
                Flow(FlowAction::Terms(TermsAction::Save)),
            Flow(FlowAction::Terms(TermsAction::Deadline)) =>
                Flow(FlowAction::Terms(TermsAction::Deadline)),
            Flow(FlowAction::Terms(TermsAction::Penalty)) =>
                Flow(FlowAction::Terms(TermsAction::Penalty)),
            Flow(FlowAction::Terms(TermsAction::Period)) =>
                Flow(FlowAction::Terms(TermsAction::Period)),
            Flow(FlowAction::Terms(TermsAction::Periods)) =>
                Flow(FlowAction::Terms(TermsAction::Periods)),
            Flow(FlowAction::Terms(TermsAction::Policy)) =>
                Flow(FlowAction::Terms(TermsAction::Policy)),
            Flow(FlowAction::Terms(TermsAction::Back)) =>
                Flow(FlowAction::Terms(TermsAction::Back)),
            Flow(FlowAction::Terms(TermsAction::DurationDate)) =>
                Flow(FlowAction::Terms(TermsAction::DurationDate)),
            Flow(FlowAction::Terms(TermsAction::DurationApply)) =>
                Flow(FlowAction::Terms(TermsAction::DurationApply)),
            Flow(FlowAction::Terms(TermsAction::Exit)) =>
                Flow(FlowAction::Terms(TermsAction::Exit)),
            Flow(FlowAction::Terms(TermsAction::Stay)) =>
                Flow(FlowAction::Terms(TermsAction::Stay)),
            Flow(FlowAction::Terms(TermsAction::Cancel)) =>
                Flow(FlowAction::Terms(TermsAction::Cancel)),
            Flow(FlowAction::Terms(TermsAction::Makeup)) =>
                Flow(FlowAction::Terms(TermsAction::Makeup)),
            Flow(FlowAction::ReviewScore { choice: AmountChoice::Preset(_), .. }) =>
                score(AmountChoice::Preset(most)),
            Flow(FlowAction::ReviewScore { choice: AmountChoice::Manual, .. }) =>
                score(AmountChoice::Manual),
            Flow(FlowAction::Deadline(DeadlineAction::Calendar)) =>
                Flow(FlowAction::Deadline(DeadlineAction::Calendar)),
            Flow(FlowAction::Deadline(DeadlineAction::Manual)) =>
                Flow(FlowAction::Deadline(DeadlineAction::Manual)),
            Flow(FlowAction::Deadline(DeadlineAction::Options)) =>
                Flow(FlowAction::Deadline(DeadlineAction::Options)),
            Flow(FlowAction::Deadline(DeadlineAction::Month(_))) =>
                Flow(FlowAction::Deadline(DeadlineAction::Month(month))),
            Flow(FlowAction::Deadline(DeadlineAction::Pick(_))) =>
                Flow(FlowAction::Deadline(DeadlineAction::Pick(day))),
            Flow(FlowAction::SelectStudent(_)) => Flow(FlowAction::SelectStudent(student)),
            Flow(FlowAction::ExtraAttempts(AmountChoice::Preset(_))) =>
                Flow(FlowAction::ExtraAttempts(AmountChoice::Preset(most))),
            Flow(FlowAction::ExtraAttempts(AmountChoice::Manual)) =>
                Flow(FlowAction::ExtraAttempts(AmountChoice::Manual)),
            Flow(FlowAction::ConfirmFileDeletion) => Flow(FlowAction::ConfirmFileDeletion),
            Flow(FlowAction::CancelFileDeletion) => Flow(FlowAction::CancelFileDeletion),
            Flow(FlowAction::ConfirmInviteRotation) => Flow(FlowAction::ConfirmInviteRotation),
            Flow(FlowAction::CancelInviteRotation) => Flow(FlowAction::CancelInviteRotation),
            Flow(FlowAction::RegistrationGroup(_)) => Flow(FlowAction::RegistrationGroup(group)),
            Flow(FlowAction::AssignmentGroup(_)) => Flow(FlowAction::AssignmentGroup(group)),
            Flow(FlowAction::AssignmentGroupsDone) => Flow(FlowAction::AssignmentGroupsDone),
            Flow(FlowAction::MakeupAnswer(MakeupChoice::Credit(_))) =>
                Flow(FlowAction::MakeupAnswer(MakeupChoice::Credit(most))),
            Flow(FlowAction::MakeupAnswer(MakeupChoice::Manual)) =>
                Flow(FlowAction::MakeupAnswer(MakeupChoice::Manual)),
            Flow(FlowAction::MakeupAnswer(MakeupChoice::NotOffered)) =>
                Flow(FlowAction::MakeupAnswer(MakeupChoice::NotOffered)),
            Flow(FlowAction::MakeupAnswer(MakeupChoice::PassFail)) =>
                Flow(FlowAction::MakeupAnswer(MakeupChoice::PassFail)),
            Flow(FlowAction::MakeupAttempts(AmountChoice::Preset(_))) =>
                Flow(FlowAction::MakeupAttempts(AmountChoice::Preset(most))),
            Flow(FlowAction::MakeupAttempts(AmountChoice::Manual)) =>
                Flow(FlowAction::MakeupAttempts(AmountChoice::Manual)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Date)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Date)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Attempts)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Attempts)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Disable)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Disable)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Save)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Save)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Cancel)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Cancel)),
            Flow(FlowAction::MakeupForm(MakeupFormAction::Back)) =>
                Flow(FlowAction::MakeupForm(MakeupFormAction::Back)),
            Informational(InformationalAction::StudentContact { .. }) =>
                Informational(InformationalAction::StudentContact {
                    course_id: course,
                    student_id: student,
                }),
            CallbackAction::Noop => CallbackAction::Noop,
        ]
    }

    #[test]
    fn every_action_is_parsed_back_from_its_own_callback() {
        for action in one_sample_of_each_action() {
            let data = action.callback();
            assert!(data.len() <= 64, "{data}");
            assert_eq!(parse(&data), Some(action), "{data}");
        }
    }

    #[derive(Clone, Debug)]
    enum Edit {
        Cut(usize),
        Replace(usize, char),
        Insert(usize, String),
    }

    fn edit() -> impl Strategy<Value = Edit> {
        let character = prop_oneof![
            prop::sample::select(vec![
                ':', '-', '_', '=', '+', '/', ' ', '0', '9', 'a', 'Z', 'я', '🦀'
            ]),
            any::<char>(),
        ];
        prop_oneof![
            any::<usize>().prop_map(Edit::Cut),
            (any::<usize>(), character.clone())
                .prop_map(|(at, character)| Edit::Replace(at, character)),
            (any::<usize>(), prop::collection::vec(character, 1..12))
                .prop_map(|(at, text)| Edit::Insert(at, text.into_iter().collect())),
        ]
    }

    fn apply(data: &str, edits: Vec<Edit>) -> String {
        let mut characters: Vec<char> = data.chars().collect();
        for edit in edits {
            let position = |at: usize| at % (characters.len() + 1);
            match edit {
                Edit::Cut(at) => characters.truncate(position(at)),
                Edit::Replace(at, character) => {
                    if !characters.is_empty() {
                        let index = at % characters.len();
                        characters[index] = character;
                    }
                }
                Edit::Insert(at, text) => {
                    let index = position(at);
                    characters.splice(index..index, text.chars());
                }
            }
        }
        characters.into_iter().collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 1024,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn edited_callback_data_never_panics_and_parses_only_into_an_action_it_can_encode(
            seed in prop::sample::select(one_sample_of_each_action()),
            edits in prop::collection::vec(edit(), 1..4),
        ) {
            let data = apply(&seed.callback(), edits);

            if let Some(action) = parse(&data) {
                prop_assert_eq!(parse(&action.callback()), Some(action), "{}", data);
            }
        }
    }

    #[test]
    fn representative_buttons_keep_their_wire_format() {
        let id = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let course = TeachingCourseId::from_uuid(id);
        let assignment = AssignmentId::from_uuid(id);
        let submission = SubmissionId::from_uuid(id);
        let day = NaiveDate::from_ymd_opt(2026, 9, 21).unwrap();
        let month = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();

        for (action, data) in [
            (
                Replayable(ReplayableAction::Page(PageTarget::Assignments, 0)),
                "menu:assignments",
            ),
            (
                Replayable(ReplayableAction::Page(PageTarget::Course(course), 0)),
                "course:00000000-0000-0000-0000-000000000001",
            ),
            (
                Replayable(ReplayableAction::Page(
                    PageTarget::Gradebook(course, StudentGroupId::from_uuid(other)),
                    3,
                )),
                "pg:g:AAAAAAAAAAAAAAAAAAAAAQ:AAAAAAAAAAAAAAAAAAAAAg:3",
            ),
            (
                Replayable(ReplayableAction::Assignment(assignment)),
                "assignment:00000000-0000-0000-0000-000000000001",
            ),
            (
                Replayable(ReplayableAction::ManageAssignment(assignment)),
                "manage:00000000-0000-0000-0000-000000000001",
            ),
            (Screen(ScreenAction::OpenFile(2)), "file:open:2"),
            (
                Screen(ScreenAction::BulkReview(ReviewDecision::Reject)),
                "sandbox:reject-all",
            ),
            (
                Flow(FlowAction::Review {
                    decision: ReviewDecision::Accept,
                    submission_id: submission,
                }),
                "review:accept:00000000-0000-0000-0000-000000000001",
            ),
            (
                Flow(FlowAction::ReviewScore {
                    submission_id: submission,
                    choice: AmountChoice::Preset(5),
                }),
                "reviewscore:00000000-0000-0000-0000-000000000001:5",
            ),
            (
                Flow(FlowAction::MaxAttempts(AmountChoice::Preset(3))),
                "maxattempts:3",
            ),
            (
                Flow(FlowAction::ExtraAttempts(AmountChoice::Manual)),
                "attempt:manual",
            ),
            (
                Flow(FlowAction::Grading(GradingChoice::PassFail)),
                "grading:passfail",
            ),
            (
                Flow(FlowAction::LatePolicy(LatePolicyChoice::Strict)),
                "late:none",
            ),
            (
                Flow(FlowAction::LatePolicy(LatePolicyChoice::Penalty(
                    DeductedFrom::CourseTotal,
                ))),
                "late:course_total",
            ),
            (
                Flow(FlowAction::Deadline(DeadlineAction::Pick(day))),
                "cal:pick:2026-09-21",
            ),
            (
                Flow(FlowAction::Deadline(DeadlineAction::Month(month))),
                "cal:2026-09",
            ),
            (
                Flow(FlowAction::MakeupAnswer(MakeupChoice::Credit(6))),
                "makeup-answer:6",
            ),
            (Flow(FlowAction::Terms(TermsAction::Save)), "terms:save"),
            (
                Flow(FlowAction::SelectStudent(UserId::from_uuid(id))),
                "student:00000000-0000-0000-0000-000000000001",
            ),
            (
                Informational(InformationalAction::StudentContact {
                    course_id: course,
                    student_id: UserId::from_uuid(other),
                }),
                "contact:AAAAAAAAAAAAAAAAAAAAAQ:AAAAAAAAAAAAAAAAAAAAAg",
            ),
            (CallbackAction::Noop, "cal:noop"),
        ] {
            assert_eq!(action.callback(), data);
            assert_eq!(parse(data), Some(action), "{data}");
        }
    }

    #[test]
    fn malformed_callbacks_do_not_fall_into_a_broader_route() {
        for invalid in [
            "archive:not-a-uuid",
            "file:open:nope",
            "maxattempts:+1",
            "grading:none",
            "score:0",
            "reviewscore:0",
            "reviewscore:not-a-uuid:0",
            "review:change:not-a-uuid",
            "late:penalty",
            "late:manual",
            "late:7:1",
            "late:7:1:4",
            "late:7:1:4:9",
            "cal:pick:2026-02-30",
            "cal:2026-13",
            "makeup-answer:",
            "makeup-answer:-1",
            "makeup-form:enable",
            "makeup:grant",
            "reg-group:done",
            "agroup:all",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }
}
