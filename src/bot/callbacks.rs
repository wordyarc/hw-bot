use chrono::NaiveDate;

use crate::domain::{AssignmentId, ReviewDecision, SubmissionId, TeachingCourseId, UserId};

pub const MENU_ASSIGNMENTS: &str = "menu:assignments";
pub const MENU_ARCHIVE: &str = "menu:archive";
pub const MENU_QUEUE: &str = "menu:queue";
pub const MENU_PROFILE: &str = "menu:profile";
pub const MENU_COURSES: &str = "menu:courses";
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
pub const PUBLISH_PREFIX: &str = "publish:";
pub const ARCHIVE_PREFIX: &str = "archive:";
pub const DEADLINE_PREFIX: &str = "deadline:";
pub const STUDENT_PREFIX: &str = "student:";
pub const MAX_ATTEMPTS_PREFIX: &str = "maxattempts:";
pub const EXTRA_ATTEMPTS_PREFIX: &str = "attempt:";
pub const GRADING_PREFIX: &str = "grading:";
pub const REVIEW_SCORE_PREFIX: &str = "reviewscore:";
pub const LATE_POLICY_PREFIX: &str = "late:";

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

pub const DEADLINE_CALENDAR: &str = "dl:calendar";
pub const DEADLINE_MANUAL: &str = "dl:manual";
pub const DEADLINE_OPTIONS: &str = "dl:options";
pub const DEADLINE_DAYS_PREFIX: &str = "dl:days:";
pub const DEADLINE_MONTHS_PREFIX: &str = "dl:months:";
pub const CALENDAR_PICK_PREFIX: &str = "cal:pick:";
/// Разбирается последним из семейства `cal:`, иначе съел бы `cal:pick:` и `cal:noop`
pub const CALENDAR_MONTH_PREFIX: &str = "cal:";
pub const CALENDAR_NOOP: &str = "cal:noop";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CallbackAction {
    Replayable(ReplayableAction),
    Screen(ScreenAction),
    Flow(FlowAction),
    Noop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplayableAction {
    StudentAssignments,
    StudentArchive,
    StudentQueue,
    Profile,
    Courses,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FlowAction {
    Review {
        decision: ReviewDecision,
        submission_id: SubmissionId,
    },
    MaxAttempts(AmountChoice),
    Grading(GradingChoice),
    LatePolicy(LatePolicyChoice),
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
    None,
    Preset {
        period_days: u16,
        penalty: u16,
        max_periods: u16,
    },
    Manual,
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

type ReplayableFamily = (&'static str, fn(uuid::Uuid) -> ReplayableAction);

// Порядок значим: выигрывает первый подошедший префикс.
const REPLAYABLE_FAMILIES: [ReplayableFamily; 9] = [
    (COURSE_PREFIX, |id| {
        ReplayableAction::Course(TeachingCourseId::from_uuid(id))
    }),
    (ASSIGNMENT_PREFIX, |id| {
        ReplayableAction::Assignment(AssignmentId::from_uuid(id))
    }),
    (MANAGE_PREFIX, |id| {
        ReplayableAction::ManageAssignment(AssignmentId::from_uuid(id))
    }),
    (CREATE_PREFIX, |id| {
        ReplayableAction::StartAssignmentCreation(TeachingCourseId::from_uuid(id))
    }),
    (SUBMIT_PREFIX, |id| {
        ReplayableAction::StartSubmission(AssignmentId::from_uuid(id))
    }),
    (FILES_PREFIX, |id| {
        ReplayableAction::AssignmentFiles(AssignmentId::from_uuid(id))
    }),
    (ATTEMPTS_PREFIX, |id| {
        ReplayableAction::GrantAttempts(AssignmentId::from_uuid(id))
    }),
    (INVITE_PREFIX, |id| {
        ReplayableAction::GroupInvite(TeachingCourseId::from_uuid(id))
    }),
    (GRADES_PREFIX, |id| {
        ReplayableAction::Gradebook(TeachingCourseId::from_uuid(id))
    }),
];

pub(super) fn parse(data: &str) -> Option<CallbackAction> {
    let replayable = match data {
        MENU_ASSIGNMENTS => Some(ReplayableAction::StudentAssignments),
        MENU_ARCHIVE => Some(ReplayableAction::StudentArchive),
        MENU_QUEUE => Some(ReplayableAction::StudentQueue),
        MENU_PROFILE => Some(ReplayableAction::Profile),
        MENU_COURSES => Some(ReplayableAction::Courses),
        NEXT_REVIEW => Some(ReplayableAction::NextReview),
        _ => REPLAYABLE_FAMILIES
            .iter()
            .find_map(|(prefix, action)| parse_uuid(data, prefix).map(action)),
    };
    if let Some(action) = replayable {
        return Some(CallbackAction::Replayable(action));
    }

    let screen = parse_uuid(data, PUBLISH_PREFIX)
        .map(AssignmentId::from_uuid)
        .map(ScreenAction::Publish)
        .or_else(|| {
            parse_uuid(data, ARCHIVE_PREFIX)
                .map(AssignmentId::from_uuid)
                .map(ScreenAction::Archive)
        })
        .or_else(|| {
            parse_uuid(data, DEADLINE_PREFIX)
                .map(AssignmentId::from_uuid)
                .map(ScreenAction::ChangeDeadline)
        })
        .or_else(|| match data {
            FILE_ADD => Some(ScreenAction::AddFile),
            FILE_FINISH => Some(ScreenAction::FinishFileCreation),
            INVITE_ROTATE => Some(ScreenAction::RotateGroupInvite),
            _ => data
                .strip_prefix(FILE_OPEN_PREFIX)
                .and_then(|index| index.parse().ok())
                .map(ScreenAction::OpenFile)
                .or_else(|| {
                    data.strip_prefix(FILE_DELETE_PREFIX)
                        .and_then(|index| index.parse().ok())
                        .map(ScreenAction::DeleteFile)
                }),
        });
    if let Some(action) = screen {
        return Some(CallbackAction::Screen(action));
    }

    let flow = parse_review(data)
        .or_else(|| parse_amount(data, MAX_ATTEMPTS_PREFIX).map(FlowAction::MaxAttempts))
        .or_else(|| parse_grading(data).map(FlowAction::Grading))
        .or_else(|| parse_late_policy(data).map(FlowAction::LatePolicy))
        .or_else(|| parse_review_score(data))
        .or_else(|| parse_deadline(data).map(FlowAction::Deadline))
        .or_else(|| {
            parse_uuid(data, STUDENT_PREFIX)
                .map(UserId::from_uuid)
                .map(FlowAction::SelectStudent)
        })
        .or_else(|| parse_amount(data, EXTRA_ATTEMPTS_PREFIX).map(FlowAction::ExtraAttempts))
        .or(match data {
            FILE_CONFIRM_DELETE => Some(FlowAction::ConfirmFileDeletion),
            FILE_CANCEL_DELETE => Some(FlowAction::CancelFileDeletion),
            INVITE_CONFIRM_ROTATE => Some(FlowAction::ConfirmInviteRotation),
            INVITE_CANCEL_ROTATE => Some(FlowAction::CancelInviteRotation),
            _ => None,
        });
    if let Some(action) = flow {
        return Some(CallbackAction::Flow(action));
    }

    (data == CALENDAR_NOOP).then_some(CallbackAction::Noop)
}

fn parse_uuid(data: &str, prefix: &str) -> Option<uuid::Uuid> {
    data.strip_prefix(prefix)?.parse().ok()
}

fn parse_review(data: &str) -> Option<FlowAction> {
    let (decision, id) = data
        .strip_prefix(REVIEW_ACCEPT_PREFIX)
        .map(|id| (ReviewDecision::Accept, id))
        .or_else(|| {
            data.strip_prefix(REVIEW_REJECT_PREFIX)
                .map(|id| (ReviewDecision::Reject, id))
        })?;

    Some(FlowAction::Review {
        decision,
        submission_id: SubmissionId::from_uuid(id.parse().ok()?),
    })
}

fn parse_review_score(data: &str) -> Option<FlowAction> {
    let value = data.strip_prefix(REVIEW_SCORE_PREFIX)?;
    let (submission_id, choice) = value.split_once(':')?;
    let submission_id = SubmissionId::from_uuid(submission_id.parse().ok()?);
    let choice = parse_amount(choice, "")?;

    Some(FlowAction::ReviewScore {
        submission_id,
        choice,
    })
}

fn parse_grading(data: &str) -> Option<GradingChoice> {
    match data.strip_prefix(GRADING_PREFIX)? {
        "passfail" => Some(GradingChoice::PassFail),
        "manual" => Some(GradingChoice::Manual),
        value => match parse_amount(data, GRADING_PREFIX)? {
            AmountChoice::Preset(max_score) => Some(GradingChoice::Preset(max_score)),
            AmountChoice::Manual => unreachable!("{value} is not the manual marker"),
        },
    }
}

fn parse_late_policy(data: &str) -> Option<LatePolicyChoice> {
    let value = data.strip_prefix(LATE_POLICY_PREFIX)?;
    match value {
        "none" => Some(LatePolicyChoice::None),
        "manual" => Some(LatePolicyChoice::Manual),
        _ => {
            let mut parts = value.split(':');
            let mut next = || parts.next()?.parse().ok();
            let (period_days, penalty, max_periods) = (next()?, next()?, next()?);

            parts.next().is_none().then_some(LatePolicyChoice::Preset {
                period_days,
                penalty,
                max_periods,
            })
        }
    }
}

fn parse_amount(data: &str, prefix: &str) -> Option<AmountChoice> {
    let value = data.strip_prefix(prefix)?;
    if value == "manual" {
        return Some(AmountChoice::Manual);
    }
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    value.parse().ok().map(AmountChoice::Preset)
}

fn parse_deadline(data: &str) -> Option<DeadlineAction> {
    match data {
        DEADLINE_CALENDAR => Some(DeadlineAction::Calendar),
        DEADLINE_MANUAL => Some(DeadlineAction::Manual),
        DEADLINE_OPTIONS => Some(DeadlineAction::Options),
        _ => data
            .strip_prefix(DEADLINE_DAYS_PREFIX)
            .and_then(|days| days.parse().ok())
            .map(DeadlineAction::Days)
            .or_else(|| {
                data.strip_prefix(DEADLINE_MONTHS_PREFIX)
                    .and_then(|months| months.parse().ok())
                    .map(DeadlineAction::Months)
            })
            .or_else(|| {
                data.strip_prefix(CALENDAR_PICK_PREFIX)
                    .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
                    .map(DeadlineAction::Pick)
            })
            .or_else(|| {
                data.strip_prefix(CALENDAR_MONTH_PREFIX)
                    .and_then(parse_month)
                    .map(DeadlineAction::Month)
            }),
    }
}

fn parse_month(value: &str) -> Option<NaiveDate> {
    let (year, month) = value.split_once('-')?;
    NaiveDate::from_ymd_opt(year.parse().ok()?, month.parse().ok()?, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            (format!("course:{course}"), "replayable"),
            (format!("assignment:{assignment}"), "replayable"),
            (format!("manage:{assignment}"), "replayable"),
            (format!("create:{course}"), "replayable"),
            (format!("submit:{assignment}"), "replayable"),
            (format!("files:{assignment}"), "replayable"),
            (format!("attempts:{assignment}"), "replayable"),
            (format!("invite:{course}"), "replayable"),
            (format!("grades:{course}"), "replayable"),
            (format!("publish:{assignment}"), "screen"),
            (format!("archive:{assignment}"), "screen"),
            (format!("deadline:{assignment}"), "screen"),
            ("file:add".to_owned(), "screen"),
            ("file:open:0".to_owned(), "screen"),
            ("file:delete:0".to_owned(), "screen"),
            ("file:finish".to_owned(), "screen"),
            ("invite:rotate".to_owned(), "screen"),
            (format!("review:accept:{submission}"), "flow"),
            ("maxattempts:3".to_owned(), "flow"),
            ("grading:passfail".to_owned(), "flow"),
            ("grading:5".to_owned(), "flow"),
            ("grading:manual".to_owned(), "flow"),
            (format!("reviewscore:{submission}:0"), "flow"),
            ("late:none".to_owned(), "flow"),
            ("late:7:1:4".to_owned(), "flow"),
            ("late:manual".to_owned(), "flow"),
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
                Some(CallbackAction::Noop) => "noop",
                None => "invalid",
            };
            assert_eq!(actual, expected, "{data}");
        }
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
