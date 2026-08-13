use chrono::NaiveDate;

use crate::domain::{AssignmentId, ReviewDecision, SubmissionId, TeachingCourseId, UserId};

pub(super) const MAX_ATTEMPTS_PREFIX: &str = "maxattempts:";
pub(super) const EXTRA_ATTEMPTS_PREFIX: &str = "attempt:";

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FlowAction {
    SelectRegistrationGroup(usize),
    Review {
        decision: ReviewDecision,
        submission_id: SubmissionId,
    },
    MaxAttempts(AmountChoice),
    Deadline(DeadlineAction),
    SelectStudent(UserId),
    ExtraAttempts(AmountChoice),
    ConfirmFileDeletion,
    CancelFileDeletion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AmountChoice {
    Preset(u16),
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

pub(super) fn parse(data: &str) -> Option<CallbackAction> {
    let replayable = match data {
        "menu:assignments" => Some(ReplayableAction::StudentAssignments),
        "menu:archive" => Some(ReplayableAction::StudentArchive),
        "menu:queue" => Some(ReplayableAction::StudentQueue),
        "menu:profile" => Some(ReplayableAction::Profile),
        "menu:courses" => Some(ReplayableAction::Courses),
        "review:next" => Some(ReplayableAction::NextReview),
        _ => parse_uuid(data, "course:")
            .map(TeachingCourseId::from_uuid)
            .map(ReplayableAction::Course)
            .or_else(|| {
                parse_uuid(data, "assignment:")
                    .map(AssignmentId::from_uuid)
                    .map(ReplayableAction::Assignment)
            })
            .or_else(|| {
                parse_uuid(data, "manage:")
                    .map(AssignmentId::from_uuid)
                    .map(ReplayableAction::ManageAssignment)
            })
            .or_else(|| {
                parse_uuid(data, "create:")
                    .map(TeachingCourseId::from_uuid)
                    .map(ReplayableAction::StartAssignmentCreation)
            })
            .or_else(|| {
                parse_uuid(data, "submit:")
                    .map(AssignmentId::from_uuid)
                    .map(ReplayableAction::StartSubmission)
            })
            .or_else(|| {
                parse_uuid(data, "files:")
                    .map(AssignmentId::from_uuid)
                    .map(ReplayableAction::AssignmentFiles)
            })
            .or_else(|| {
                parse_uuid(data, "attempts:")
                    .map(AssignmentId::from_uuid)
                    .map(ReplayableAction::GrantAttempts)
            }),
    };
    if let Some(action) = replayable {
        return Some(CallbackAction::Replayable(action));
    }

    let screen = parse_uuid(data, "publish:")
        .map(AssignmentId::from_uuid)
        .map(ScreenAction::Publish)
        .or_else(|| {
            parse_uuid(data, "archive:")
                .map(AssignmentId::from_uuid)
                .map(ScreenAction::Archive)
        })
        .or_else(|| {
            parse_uuid(data, "deadline:")
                .map(AssignmentId::from_uuid)
                .map(ScreenAction::ChangeDeadline)
        })
        .or_else(|| match data {
            "file:add" => Some(ScreenAction::AddFile),
            "file:finish" => Some(ScreenAction::FinishFileCreation),
            _ => data
                .strip_prefix("file:open:")
                .and_then(|index| index.parse().ok())
                .map(ScreenAction::OpenFile)
                .or_else(|| {
                    data.strip_prefix("file:delete:")
                        .and_then(|index| index.parse().ok())
                        .map(ScreenAction::DeleteFile)
                }),
        });
    if let Some(action) = screen {
        return Some(CallbackAction::Screen(action));
    }

    let flow = data
        .strip_prefix("register_group:")
        .and_then(|index| index.parse().ok())
        .map(FlowAction::SelectRegistrationGroup)
        .or_else(|| parse_review(data))
        .or_else(|| parse_amount(data, MAX_ATTEMPTS_PREFIX).map(FlowAction::MaxAttempts))
        .or_else(|| parse_deadline(data).map(FlowAction::Deadline))
        .or_else(|| {
            parse_uuid(data, "student:")
                .map(UserId::from_uuid)
                .map(FlowAction::SelectStudent)
        })
        .or_else(|| parse_amount(data, EXTRA_ATTEMPTS_PREFIX).map(FlowAction::ExtraAttempts))
        .or(match data {
            "file:confirm-delete" => Some(FlowAction::ConfirmFileDeletion),
            "file:cancel-delete" => Some(FlowAction::CancelFileDeletion),
            _ => None,
        });
    if let Some(action) = flow {
        return Some(CallbackAction::Flow(action));
    }

    (data == "cal:noop").then_some(CallbackAction::Noop)
}

fn parse_uuid(data: &str, prefix: &str) -> Option<uuid::Uuid> {
    data.strip_prefix(prefix)?.parse().ok()
}

fn parse_review(data: &str) -> Option<FlowAction> {
    let (decision, id) = data
        .strip_prefix("review:accept:")
        .map(|id| (ReviewDecision::Accept, id))
        .or_else(|| {
            data.strip_prefix("review:reject:")
                .map(|id| (ReviewDecision::Reject, id))
        })?;

    Some(FlowAction::Review {
        decision,
        submission_id: SubmissionId::from_uuid(id.parse().ok()?),
    })
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
        "dl:calendar" => Some(DeadlineAction::Calendar),
        "dl:manual" => Some(DeadlineAction::Manual),
        "dl:options" => Some(DeadlineAction::Options),
        _ => data
            .strip_prefix("dl:days:")
            .and_then(|days| days.parse().ok())
            .map(DeadlineAction::Days)
            .or_else(|| {
                data.strip_prefix("dl:months:")
                    .and_then(|months| months.parse().ok())
                    .map(DeadlineAction::Months)
            })
            .or_else(|| {
                data.strip_prefix("cal:pick:")
                    .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
                    .map(DeadlineAction::Pick)
            })
            .or_else(|| {
                data.strip_prefix("cal:")
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
            (format!("publish:{assignment}"), "screen"),
            (format!("archive:{assignment}"), "screen"),
            (format!("deadline:{assignment}"), "screen"),
            ("file:add".to_owned(), "screen"),
            ("file:open:0".to_owned(), "screen"),
            ("file:delete:0".to_owned(), "screen"),
            ("file:finish".to_owned(), "screen"),
            ("register_group:0".to_owned(), "flow"),
            (format!("review:accept:{submission}"), "flow"),
            ("maxattempts:3".to_owned(), "flow"),
            ("dl:calendar".to_owned(), "flow"),
            (format!("student:{student}"), "flow"),
            ("attempt:3".to_owned(), "flow"),
            ("file:confirm-delete".to_owned(), "flow"),
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
        assert_eq!(parse("dl:days:nope"), None);
        assert_eq!(parse("cal:2026-13"), None);
    }
}
