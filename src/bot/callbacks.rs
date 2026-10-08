use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::NaiveDate;
use uuid::Uuid;

use super::handlers::input::parse_u16;
use crate::domain::{
    AssignmentId, DeductedFrom, ReviewDecision, SubmissionId, TeachingCourseId, UserId,
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
pub const TERMS_EXIT: &str = "terms:exit";
pub const TERMS_STAY: &str = "terms:stay";
pub const LATE_PERIODS_PREFIX: &str = "periods:";
pub const TERMS_CANCEL: &str = "terms:cancel";

pub const FILE_ADD: &str = "file:add";
pub const FILE_FINISH: &str = "file:finish";
pub const FILE_CONFIRM_DELETE: &str = "file:confirm-delete";
pub const FILE_CANCEL_DELETE: &str = "file:cancel-delete";
pub const FILE_OPEN_PREFIX: &str = "file:open:";
pub const FILE_DELETE_PREFIX: &str = "file:delete:";

pub const INVITE_ROTATE: &str = "invite:rotate";
pub const INVITE_CONFIRM_ROTATE: &str = "invite:confirm-rotate";
pub const INVITE_CANCEL_ROTATE: &str = "invite:cancel-rotate";

pub const REVIEW_ACCEPT_PREFIX: &str = "review:accept:";
pub const REVIEW_REJECT_PREFIX: &str = "review:reject:";
pub const REVIEW_OPEN_PREFIX: &str = "review:open:";
pub const REVIEW_CHANGE_PREFIX: &str = "review:change:";
pub const SANDBOX_ACCEPT_ALL: &str = "sandbox:accept-all";
pub const SANDBOX_REJECT_ALL: &str = "sandbox:reject-all";

pub const DEADLINE_CALENDAR: &str = "dl:calendar";
pub const DEADLINE_MANUAL: &str = "dl:manual";
pub const DEADLINE_OPTIONS: &str = "dl:options";
pub const DEADLINE_DAYS_PREFIX: &str = "dl:days:";
pub const DEADLINE_MONTHS_PREFIX: &str = "dl:months:";
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
    NextReview,
    Course(TeachingCourseId),
    Assignment(AssignmentId),
    ManageAssignment(AssignmentId),
    StartAssignmentCreation(TeachingCourseId),
    StartSubmission(AssignmentId),
    AssignmentFiles(AssignmentId),
    GrantAttempts(AssignmentId),
    GroupInvite(TeachingCourseId),
    Gradebook(TeachingCourseId),
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
            Self::Page(target, page) => target.callback(page),
            Self::Profile => MENU_PROFILE.to_owned(),
            Self::NextReview => NEXT_REVIEW.to_owned(),
            Self::Course(id) => format!("{COURSE_PREFIX}{}", id.as_uuid()),
            Self::Assignment(id) => format!("{ASSIGNMENT_PREFIX}{}", id.as_uuid()),
            Self::ManageAssignment(id) => format!("{MANAGE_PREFIX}{}", id.as_uuid()),
            Self::StartAssignmentCreation(id) => format!("{CREATE_PREFIX}{}", id.as_uuid()),
            Self::StartSubmission(id) => format!("{SUBMIT_PREFIX}{}", id.as_uuid()),
            Self::AssignmentFiles(id) => format!("{FILES_PREFIX}{}", id.as_uuid()),
            Self::GrantAttempts(id) => format!("{ATTEMPTS_PREFIX}{}", id.as_uuid()),
            Self::GroupInvite(id) => format!("{INVITE_PREFIX}{}", id.as_uuid()),
            Self::Gradebook(id) => format!("{GRADES_PREFIX}{}", id.as_uuid()),
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
    Gradebook(TeachingCourseId),
    History(AssignmentId),
    StudentGrades(TeachingCourseId, UserId),
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
            Self::Gradebook(course) => format!("g:{}", id(course.as_uuid())),
            Self::History(assignment) => format!("h:{}", id(assignment.as_uuid())),
            Self::StudentGrades(course, student) => {
                format!("sg:{}:{}", id(course.as_uuid()), id(student.as_uuid()))
            }
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
        "g" => PageTarget::Gradebook(TeachingCourseId::from_uuid(id()?)),
        "h" => PageTarget::History(AssignmentId::from_uuid(id()?)),
        "sg" => {
            PageTarget::StudentGrades(TeachingCourseId::from_uuid(id()?), UserId::from_uuid(id()?))
        }
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
    RotateGroupInvite,
    BulkReview(ReviewDecision),
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
            Self::RotateGroupInvite => INVITE_ROTATE.to_owned(),
            Self::BulkReview(ReviewDecision::Accept) => SANDBOX_ACCEPT_ALL.to_owned(),
            Self::BulkReview(ReviewDecision::Reject) => SANDBOX_REJECT_ALL.to_owned(),
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AmountChoice {
    Preset(u16),
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GradingChoice {
    PassFail,
    Preset(u16),
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LatePolicyChoice {
    Strict,
    Penalty(DeductedFrom),
}

impl LatePolicyChoice {
    pub fn callback(self) -> String {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeadlineAction {
    Days(u32),
    Months(u32),
    Calendar,
    Manual,
    Options,
    Month(NaiveDate),
    Pick(NaiveDate),
}

pub(super) fn parse(data: &str) -> Option<CallbackAction> {
    let literal = match data {
        MENU_ASSIGNMENTS => Replayable(ReplayableAction::Page(PageTarget::Assignments, 0)),
        MENU_ARCHIVE => Replayable(ReplayableAction::Page(PageTarget::Archive, 0)),
        MENU_QUEUE => Replayable(ReplayableAction::Page(PageTarget::Queue, 0)),
        MENU_PROFILE => Replayable(ReplayableAction::Profile),
        MENU_COURSES => Replayable(ReplayableAction::Page(PageTarget::Courses, 0)),
        MENU_SANDBOX => Replayable(ReplayableAction::Page(PageTarget::Sandbox, 0)),
        NEXT_REVIEW => Replayable(ReplayableAction::NextReview),
        FILE_ADD => Screen(ScreenAction::AddFile),
        FILE_FINISH => Screen(ScreenAction::FinishFileCreation),
        INVITE_ROTATE => Screen(ScreenAction::RotateGroupInvite),
        SANDBOX_ACCEPT_ALL => Screen(ScreenAction::BulkReview(ReviewDecision::Accept)),
        SANDBOX_REJECT_ALL => Screen(ScreenAction::BulkReview(ReviewDecision::Reject)),
        FILE_CONFIRM_DELETE => Flow(FlowAction::ConfirmFileDeletion),
        FILE_CANCEL_DELETE => Flow(FlowAction::CancelFileDeletion),
        INVITE_CONFIRM_ROTATE => Flow(FlowAction::ConfirmInviteRotation),
        INVITE_CANCEL_ROTATE => Flow(FlowAction::CancelInviteRotation),
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
        Replayable(ReplayableAction::Course(TeachingCourseId::from_uuid(id)))
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
        Replayable(ReplayableAction::GrantAttempts(AssignmentId::from_uuid(id)))
    }),
    (INVITE_PREFIX, |id| {
        Replayable(ReplayableAction::GroupInvite(TeachingCourseId::from_uuid(
            id,
        )))
    }),
    (GRADES_PREFIX, |id| {
        Replayable(ReplayableAction::Gradebook(TeachingCourseId::from_uuid(id)))
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
];

const AMOUNT_ROUTES: &[Route<AmountChoice, FlowAction>] = &[
    (MAX_ATTEMPTS_PREFIX, FlowAction::MaxAttempts),
    (EXTRA_ATTEMPTS_PREFIX, FlowAction::ExtraAttempts),
    (PENALTY_PREFIX, FlowAction::Penalty),
    (LATE_PERIODS_PREFIX, FlowAction::Periods),
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
    if data.starts_with(GRADING_PREFIX) {
        parse_grading(data).map(FlowAction::Grading)
    } else if data.starts_with(LATE_POLICY_PREFIX) {
        parse_late_policy(data).map(FlowAction::LatePolicy)
    } else if data.starts_with(REVIEW_SCORE_PREFIX) {
        parse_review_score(data)
    } else {
        parse_deadline(data).map(FlowAction::Deadline)
    }
}

fn parse_review_score(data: &str) -> Option<FlowAction> {
    let value = data.strip_prefix(REVIEW_SCORE_PREFIX)?;
    let (submission_id, choice) = value.split_once(':')?;
    let submission_id = SubmissionId::from_uuid(submission_id.parse().ok()?);
    let choice = parse_amount_value(choice)?;

    Some(FlowAction::ReviewScore {
        submission_id,
        choice,
    })
}

fn parse_grading(data: &str) -> Option<GradingChoice> {
    match data.strip_prefix(GRADING_PREFIX)? {
        "passfail" => Some(GradingChoice::PassFail),
        "manual" => Some(GradingChoice::Manual),
        value => parse_u16(value).map(GradingChoice::Preset),
    }
}

fn parse_late_policy(data: &str) -> Option<LatePolicyChoice> {
    match data.strip_prefix(LATE_POLICY_PREFIX)? {
        STRICT_LATE_POLICY => Some(LatePolicyChoice::Strict),
        value => DeductedFrom::parse_str(value).map(LatePolicyChoice::Penalty),
    }
}

fn parse_amount_value(value: &str) -> Option<AmountChoice> {
    if value == "manual" {
        return Some(AmountChoice::Manual);
    }
    parse_u16(value).map(AmountChoice::Preset)
}

fn parse_deadline(data: &str) -> Option<DeadlineAction> {
    if let Some(days) = data.strip_prefix(DEADLINE_DAYS_PREFIX) {
        return days.parse().ok().map(DeadlineAction::Days);
    }
    if let Some(months) = data.strip_prefix(DEADLINE_MONTHS_PREFIX) {
        return months.parse().ok().map(DeadlineAction::Months);
    }
    // cal: поглотил бы cal:pick:, если разобрать общий префикс первым.
    if let Some(date) = data.strip_prefix(CALENDAR_PICK_PREFIX) {
        return NaiveDate::parse_from_str(date, "%Y-%m-%d")
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
    fn pagination_routes_fit_telegram_even_with_two_ids_and_the_last_page() {
        for target in [
            PageTarget::Assignments,
            PageTarget::Archive,
            PageTarget::Queue,
            PageTarget::Courses,
            PageTarget::Sandbox,
            PageTarget::Course(TeachingCourseId::new()),
            PageTarget::Students(AssignmentId::new()),
            PageTarget::Gradebook(TeachingCourseId::new()),
            PageTarget::History(AssignmentId::new()),
            PageTarget::StudentGrades(TeachingCourseId::new(), UserId::new()),
        ] {
            let data = target.callback(u32::MAX);
            assert!(data.len() <= 64, "{data}");
            assert_eq!(
                parse(&data),
                Some(CallbackAction::Replayable(ReplayableAction::Page(
                    target,
                    u32::MAX
                )))
            );
        }
        for invalid in [
            "pg:a:-1",
            "pg:a:4294967296",
            "pg:a:extra:0",
            "pg:sg:bad:bad:0",
            "pg:a:",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn callbacks_are_split_by_state_policy() {
        let assignment_id = AssignmentId::new();
        let submission_id = SubmissionId::new();

        assert_eq!(
            parse(&format!("assignment:{}", assignment_id.as_uuid())),
            Some(CallbackAction::Replayable(ReplayableAction::Assignment(
                assignment_id
            )))
        );
        assert_eq!(
            parse(&format!("archive:{}", assignment_id.as_uuid())),
            Some(CallbackAction::Screen(ScreenAction::Archive(assignment_id)))
        );
        assert_eq!(
            parse(&format!("review:accept:{}", submission_id.as_uuid())),
            Some(CallbackAction::Flow(FlowAction::Review {
                decision: ReviewDecision::Accept,
                submission_id,
            }))
        );
        let score = format!("{REVIEW_SCORE_PREFIX}{}:5", submission_id.as_uuid());
        assert_eq!(
            parse(&score),
            Some(CallbackAction::Flow(FlowAction::ReviewScore {
                submission_id,
                choice: AmountChoice::Preset(5),
            }))
        );
        assert!(score.len() <= 64);
    }

    #[test]
    fn every_callback_family_has_an_explicit_state_policy() {
        let assignment = AssignmentId::new().as_uuid();
        let course = TeachingCourseId::new().as_uuid();
        let submission = SubmissionId::new().as_uuid();
        let student = UserId::new().as_uuid();
        let cases = [
            ("menu:assignments".to_owned(), "replayable"),
            ("menu:sandbox".to_owned(), "replayable"),
            (format!("review:open:{submission}"), "replayable"),
            (format!("course:{course}"), "replayable"),
            (format!("assignment:{assignment}"), "replayable"),
            (format!("manage:{assignment}"), "replayable"),
            (format!("create:{course}"), "replayable"),
            (format!("submit:{assignment}"), "replayable"),
            (format!("files:{assignment}"), "replayable"),
            (format!("attempts:{assignment}"), "replayable"),
            (format!("invite:{course}"), "replayable"),
            (format!("grades:{course}"), "replayable"),
            (
                format!(
                    "{CONTACT_PREFIX}{}:{}",
                    encode_id(course),
                    encode_id(student)
                ),
                "informational",
            ),
            (format!("publish:{assignment}"), "screen"),
            (format!("archive:{assignment}"), "screen"),
            (format!("deadline:{assignment}"), "screen"),
            ("file:add".to_owned(), "screen"),
            ("file:open:0".to_owned(), "screen"),
            ("file:delete:0".to_owned(), "screen"),
            ("file:finish".to_owned(), "screen"),
            ("invite:rotate".to_owned(), "screen"),
            ("sandbox:accept-all".to_owned(), "screen"),
            ("sandbox:reject-all".to_owned(), "screen"),
            (format!("review:accept:{submission}"), "flow"),
            (format!("review:change:{submission}"), "flow"),
            ("maxattempts:3".to_owned(), "flow"),
            ("grading:passfail".to_owned(), "flow"),
            ("grading:5".to_owned(), "flow"),
            ("grading:manual".to_owned(), "flow"),
            (format!("reviewscore:{submission}:0"), "flow"),
            ("late:none".to_owned(), "flow"),
            ("late:work_score".to_owned(), "flow"),
            ("late:course_total".to_owned(), "flow"),
            ("late:penalty".to_owned(), "invalid"),
            ("late:7:1:4".to_owned(), "invalid"),
            ("late:manual".to_owned(), "invalid"),
            (format!("{PENALTY_PREFIX}1"), "flow"),
            (PENALTY_PERIOD.to_owned(), "flow"),
            (TERMS_SAVE.to_owned(), "flow"),
            (TERMS_DEADLINE.to_owned(), "flow"),
            (TERMS_PERIODS.to_owned(), "flow"),
            (TERMS_CANCEL.to_owned(), "flow"),
            ("dl:calendar".to_owned(), "flow"),
            (format!("student:{student}"), "flow"),
            ("attempt:3".to_owned(), "flow"),
            ("file:confirm-delete".to_owned(), "flow"),
            ("invite:confirm-rotate".to_owned(), "flow"),
            ("invite:cancel-rotate".to_owned(), "flow"),
        ];

        for (data, expected) in cases {
            let actual = match parse(&data) {
                Some(CallbackAction::Replayable(_)) => "replayable",
                Some(CallbackAction::Screen(_)) => "screen",
                Some(CallbackAction::Flow(_)) => "flow",
                Some(CallbackAction::Informational(_)) => "informational",
                Some(CallbackAction::Noop) => "noop",
                None => "invalid",
            };
            assert_eq!(actual, expected, "{data}");
        }
    }

    #[test]
    fn changing_a_review_decision_is_a_flow_step_of_its_submission() {
        let submission_id = SubmissionId::new();
        let data = format!("{REVIEW_CHANGE_PREFIX}{}", submission_id.as_uuid());

        assert_eq!(data.len(), 50);
        assert_eq!(
            parse(&data),
            Some(CallbackAction::Flow(FlowAction::ChangeReviewDecision(
                submission_id
            )))
        );
        assert_eq!(parse(&format!("{REVIEW_CHANGE_PREFIX}not-a-uuid")), None);
    }

    #[test]
    fn late_policy_choices_name_the_deduction_and_drop_the_old_penalty_choice() {
        for (choice, data) in [
            (LatePolicyChoice::Strict, "late:none"),
            (
                LatePolicyChoice::Penalty(DeductedFrom::WorkScore),
                "late:work_score",
            ),
            (
                LatePolicyChoice::Penalty(DeductedFrom::CourseTotal),
                "late:course_total",
            ),
        ] {
            assert_eq!(choice.callback(), data);
            assert_eq!(
                parse(data),
                Some(CallbackAction::Flow(FlowAction::LatePolicy(choice)))
            );
        }
        assert_eq!(parse("late:penalty"), None);
        assert_eq!(LatePolicyChoice::Strict.deducted_from(), None);
        assert_eq!(
            LatePolicyChoice::Penalty(DeductedFrom::CourseTotal).deducted_from(),
            Some(DeductedFrom::CourseTotal)
        );
    }

    #[test]
    fn malformed_callbacks_do_not_fall_into_a_broader_route() {
        assert_eq!(parse("archive:not-a-uuid"), None);
        assert_eq!(parse("file:open:nope"), None);
        assert_eq!(parse("maxattempts:+1"), None);
        assert_eq!(parse("grading:none"), None);
        assert_eq!(parse("score:0"), None);
        assert_eq!(parse("reviewscore:0"), None);
        assert_eq!(parse("reviewscore:not-a-uuid:0"), None);
        assert_eq!(parse("late:7:1"), None);
        assert_eq!(parse("late:7:1:4:9"), None);
        assert_eq!(parse("dl:days:nope"), None);
        assert_eq!(parse("cal:2026-13"), None);
    }
}
