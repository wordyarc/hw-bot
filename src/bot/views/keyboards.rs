use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup};

use super::render::button_label;
use crate::app::{
    AssignmentStudentSummary, CourseStudent, SandboxSubmission, StudentAssignment,
    TeachingCourseSummary,
};
use crate::bot::assignment_form::{
    AssignmentTerms, CompleteTerms, TermsForm, TermsStep, TermsTarget,
};
use crate::bot::callbacks::{
    AmountChoice, CallbackAction, DeadlineAction, FlowAction, GradingChoice, InformationalAction,
    LatePolicyChoice, MakeupChoice, MakeupFormAction, PageTarget, ReplayableAction, ScreenAction,
    TermsAction,
};
use crate::bot::form::{Form, InputMode};
use crate::bot::makeup_form::{MakeupForm, MakeupStep};
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, CourseMakeup, DeductedFrom, Grading,
    ReviewDecision, Role, StoredFileMetadata, StudentGroup, StudentGroupId, SubmissionId,
    TeachingCourseId, UserId,
};

type Row = Vec<InlineKeyboardButton>;

fn button(label: impl Into<String>, data: impl Into<String>) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(label, data)
}

fn row(label: impl Into<String>, data: impl Into<String>) -> Row {
    vec![button(label, data)]
}

fn deadline_callback(action: DeadlineAction) -> String {
    FlowAction::Deadline(action).callback()
}

fn terms_callback(action: TermsAction) -> String {
    FlowAction::Terms(action).callback()
}

fn makeup_form_callback(action: MakeupFormAction) -> String {
    FlowAction::MakeupForm(action).callback()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuAction {
    Assignments,
    Archive,
    Queue,
    Review,
    Courses,
    Sandbox,
    Profile,
    Help,
}

pub(crate) fn menu_action(text: &str) -> Option<MenuAction> {
    match text.trim() {
        texts::BUTTON_ASSIGNMENTS => Some(MenuAction::Assignments),
        texts::BUTTON_ARCHIVE => Some(MenuAction::Archive),
        texts::BUTTON_QUEUE => Some(MenuAction::Queue),
        texts::BUTTON_REVIEW => Some(MenuAction::Review),
        texts::BUTTON_COURSES => Some(MenuAction::Courses),
        texts::BUTTON_SANDBOX => Some(MenuAction::Sandbox),
        texts::BUTTON_PROFILE => Some(MenuAction::Profile),
        texts::BUTTON_HELP => Some(MenuAction::Help),
        _ => None,
    }
}

pub(crate) fn main_keyboard(role: Role) -> KeyboardMarkup {
    match role {
        Role::Student { .. } => student_keyboard(),
        Role::Teacher | Role::Admin => teacher_keyboard(),
    }
}

pub(crate) fn main_menu(role: Role, pending: Option<u32>) -> InlineKeyboardMarkup {
    match role {
        Role::Student { .. } => student_menu(),
        Role::Teacher | Role::Admin => teacher_menu(pending),
    }
}

pub(crate) fn student_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![
            KeyboardButton::new(texts::BUTTON_ASSIGNMENTS),
            KeyboardButton::new(texts::BUTTON_QUEUE),
        ],
        vec![
            KeyboardButton::new(texts::BUTTON_ARCHIVE),
            KeyboardButton::new(texts::BUTTON_PROFILE),
        ],
        vec![KeyboardButton::new(texts::BUTTON_HELP)],
    ])
}

pub(crate) fn teacher_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![KeyboardButton::new(texts::BUTTON_REVIEW)],
        vec![
            KeyboardButton::new(texts::BUTTON_COURSES),
            KeyboardButton::new(texts::BUTTON_PROFILE),
        ],
        vec![
            KeyboardButton::new(texts::BUTTON_SANDBOX),
            KeyboardButton::new(texts::BUTTON_HELP),
        ],
    ])
}

fn bottom_menu(rows: Vec<Vec<KeyboardButton>>) -> KeyboardMarkup {
    KeyboardMarkup::new(rows)
        .resize_keyboard()
        .persistent()
        .input_field_placeholder(texts::MENU_PLACEHOLDER)
}

pub(crate) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(
            texts::BUTTON_ASSIGNMENTS,
            ReplayableAction::Page(PageTarget::Assignments, 0).callback(),
        ),
        vec![
            button(
                texts::BUTTON_QUEUE,
                ReplayableAction::Page(PageTarget::Queue, 0).callback(),
            ),
            button(
                texts::BUTTON_ARCHIVE,
                ReplayableAction::Page(PageTarget::Archive, 0).callback(),
            ),
        ],
        row(texts::BUTTON_PROFILE, ReplayableAction::Profile.callback()),
    ])
}

pub(crate) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label =
        pending.map_or_else(|| texts::BUTTON_REVIEW.to_owned(), texts::review_button);
    InlineKeyboardMarkup::new(vec![
        row(review_label, ReplayableAction::ReviewHub.callback()),
        row(
            texts::BUTTON_COURSES,
            ReplayableAction::Page(PageTarget::Courses, 0).callback(),
        ),
        row(texts::BUTTON_PROFILE, ReplayableAction::Profile.callback()),
    ])
}

pub(crate) fn student_assignments_keyboard(
    assignments: &[StudentAssignment],
    target: PageTarget,
    page: u32,
    next: Option<u32>,
) -> InlineKeyboardMarkup {
    let rows = assignments.iter().map(|item| {
        row(
            button_label(&texts::assignment_button(
                item.subject.name().as_str(),
                item.assignment.title().as_str(),
            )),
            ReplayableAction::Assignment(item.assignment.id()).callback(),
        )
    });
    paginated_keyboard(InlineKeyboardMarkup::new(rows), target, page, next)
}

pub(crate) fn student_assignment_keyboard(
    assignment: &Assignment,
    history: bool,
    submit: Option<&str>,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if history {
        rows.push(row(
            texts::BUTTON_HISTORY,
            PageTarget::History(assignment.id()).callback(0),
        ));
    }
    if let Some(label) = submit {
        rows.push(row(
            label,
            ReplayableAction::StartSubmission(assignment.id()).callback(),
        ));
    }
    let (back_label, back_target) = if assignment.status() == AssignmentStatus::Archived {
        (texts::BUTTON_BACK_TO_ARCHIVE, PageTarget::Archive)
    } else {
        (texts::BUTTON_BACK_TO_ASSIGNMENTS, PageTarget::Assignments)
    };
    rows.push(row(
        back_label,
        ReplayableAction::Page(back_target, 0).callback(),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn reopen_assignment_keyboard(assignment_id: AssignmentId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new([row(
        texts::BUTTON_REOPEN_ASSIGNMENT,
        ReplayableAction::Assignment(assignment_id).callback(),
    )])
}

pub(crate) fn submission_history_keyboard(
    assignment_id: AssignmentId,
    page: u32,
    next: Option<u32>,
) -> InlineKeyboardMarkup {
    paginated_keyboard(
        InlineKeyboardMarkup::new([row(
            texts::BUTTON_BACK_TO_ASSIGNMENT,
            ReplayableAction::Assignment(assignment_id).callback(),
        )]),
        PageTarget::History(assignment_id),
        page,
        next,
    )
}

pub(crate) fn review_keyboard(
    submission_id: SubmissionId,
    course_id: TeachingCourseId,
    student_id: UserId,
    accept: Option<String>,
) -> InlineKeyboardMarkup {
    let review = |decision| {
        FlowAction::Review {
            decision,
            submission_id,
        }
        .callback()
    };
    let accept = accept.map(|label| button(label, review(ReviewDecision::Accept)));
    let reject = button(texts::BUTTON_REJECT, review(ReviewDecision::Reject));
    InlineKeyboardMarkup::new(vec![
        accept.into_iter().chain([reject]).collect(),
        vec![student_contact_button(course_id, student_id)],
    ])
}

pub(crate) fn review_hub_keyboard(queue: u32, makeups: u32) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if queue > 0 {
        rows.push(row(
            texts::queue_review_button(queue),
            ReplayableAction::NextReview.callback(),
        ));
    }
    rows.push(row(
        texts::makeup_review_button(makeups),
        ReplayableAction::NextMakeupReview.callback(),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn makeup_followup_keyboard(next: bool, grant: bool) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if next {
        rows.push(row(
            texts::BUTTON_NEXT_MAKEUP,
            ReplayableAction::NextMakeupReview.callback(),
        ));
    }
    if grant {
        rows.push(row(
            texts::BUTTON_GRANT_MAKEUP,
            ScreenAction::GrantMakeupAttempt.callback(),
        ));
    }
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn review_decision_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![row(
        texts::BUTTON_CHANGE_DECISION,
        FlowAction::ChangeReviewDecision(submission_id).callback(),
    )])
}

fn student_contact_button(course_id: TeachingCourseId, student_id: UserId) -> InlineKeyboardButton {
    let target = InformationalAction::StudentContact {
        course_id,
        student_id,
    };
    button(texts::BUTTON_STUDENT_CONTACT, target.callback())
}

pub(crate) fn student_grades_keyboard(
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
    student_id: UserId,
    grant_makeup: bool,
) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![student_contact_button(course_id, student_id)]];
    if grant_makeup {
        rows.push(row(
            texts::BUTTON_GRANT_MAKEUP,
            ScreenAction::GrantMakeupAttempt.callback(),
        ));
    }
    rows.push(row(
        texts::BUTTON_BACK_TO_STUDENTS,
        ReplayableAction::Page(PageTarget::Gradebook(course_id, group_id), 0).callback(),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn contact_keyboard(
    target: InformationalAction,
    url: Option<reqwest::Url>,
    refresh: &str,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if let Some(url) = url {
        rows.push(vec![InlineKeyboardButton::url(
            texts::BUTTON_OPEN_TELEGRAM,
            url,
        )]);
    }
    rows.push(row(refresh, target.callback()));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn sandbox_keyboard(
    submissions: &[SandboxSubmission],
    page: u32,
    next: Option<u32>,
) -> InlineKeyboardMarkup {
    let rows = submissions.iter().map(|submission| {
        row(
            button_label(&texts::sandbox_button(
                &submission.student_name,
                &submission.assignment_title,
            )),
            ReplayableAction::ReviewSubmission(submission.submission_id).callback(),
        )
    });
    let mut keyboard = paginated_keyboard(
        InlineKeyboardMarkup::new(rows),
        PageTarget::Sandbox,
        page,
        next,
    );
    keyboard.inline_keyboard.push(vec![
        button(
            texts::BUTTON_ACCEPT_ALL,
            ScreenAction::BulkReview(ReviewDecision::Accept).callback(),
        ),
        button(
            texts::BUTTON_REJECT_ALL,
            ScreenAction::BulkReview(ReviewDecision::Reject).callback(),
        ),
    ]);
    keyboard
}

pub(crate) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(courses.iter().map(|course| {
        let subject = course.subject.name().as_str();
        let label = match course.groups.as_slice() {
            [group] => texts::course_button(subject, group.code().as_str()),
            groups => texts::course_groups_button(subject, groups.len()),
        };
        row(
            button_label(&label),
            ReplayableAction::Page(PageTarget::Course(course.course.id()), 0).callback(),
        )
    }))
}

pub(crate) fn course_assignments_keyboard(
    course_id: TeachingCourseId,
    assignments: &[Assignment],
    makeup: Option<CourseMakeup>,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = assignments
        .iter()
        .map(|assignment| {
            row(
                button_label(&texts::course_assignment_button(
                    texts::assignment_status_icon(assignment.status()),
                    assignment.title().as_str(),
                )),
                ReplayableAction::ManageAssignment(assignment.id()).callback(),
            )
        })
        .collect();
    rows.extend([
        row(
            texts::BUTTON_CREATE_ASSIGNMENT,
            ReplayableAction::StartAssignmentCreation(course_id).callback(),
        ),
        row(
            texts::BUTTON_GRADEBOOK,
            ReplayableAction::GradebookGroups(course_id).callback(),
        ),
        row(
            texts::BUTTON_ROSTER,
            ReplayableAction::Page(PageTarget::Roster(course_id), 0).callback(),
        ),
        row(
            makeup.map_or_else(
                || texts::BUTTON_MAKEUP_OFF.to_owned(),
                |makeup| {
                    texts::makeup_button(
                        &texts::until(&timezone.format_until_short(makeup.until())),
                        makeup.attempts(),
                    )
                },
            ),
            ReplayableAction::CourseMakeup(course_id).callback(),
        ),
        row(
            texts::BUTTON_COURSE_INVITE,
            ReplayableAction::CourseInvite(course_id).callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSES,
            ReplayableAction::Page(PageTarget::Courses, 0).callback(),
        ),
    ]);

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn assignment_actions_keyboard(assignment: &Assignment) -> InlineKeyboardMarkup {
    let assignment_id = assignment.id();
    let publish = |label| row(label, ScreenAction::Publish(assignment_id).callback());
    let archive = || {
        row(
            texts::BUTTON_ARCHIVE_ASSIGNMENT,
            ScreenAction::Archive(assignment_id).callback(),
        )
    };

    let mut rows = match assignment.status() {
        AssignmentStatus::Draft => vec![publish(texts::BUTTON_PUBLISH), archive()],
        AssignmentStatus::Published => vec![archive()],
        AssignmentStatus::Archived => vec![publish(texts::BUTTON_PUBLISH_AGAIN)],
    };
    if assignment.status() != AssignmentStatus::Archived {
        rows.push(row(
            texts::BUTTON_EDIT_DATES,
            ScreenAction::ChangeDeadline(assignment_id).callback(),
        ));
    }
    rows.extend([
        row(
            texts::BUTTON_FILES,
            ReplayableAction::AssignmentFiles(assignment_id).callback(),
        ),
        row(
            texts::BUTTON_GRANT_ATTEMPTS,
            ReplayableAction::Page(PageTarget::Students(assignment_id), 0).callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSE_ASSIGNMENTS,
            ReplayableAction::Page(PageTarget::Course(assignment.teaching_course_id()), 0)
                .callback(),
        ),
    ]);

    InlineKeyboardMarkup::new(rows)
}

fn date_input_rows() -> [Row; 2] {
    [
        row(
            texts::BUTTON_CALENDAR,
            deadline_callback(DeadlineAction::Calendar),
        ),
        row(
            texts::BUTTON_MANUAL,
            deadline_callback(DeadlineAction::Manual),
        ),
    ]
}

pub(crate) fn deadline_options_keyboard(timezone: &BotTimezone) -> InlineKeyboardMarkup {
    let preset = |label: &str, date: Option<NaiveDate>, action: DeadlineAction| {
        date.map(|date| {
            row(
                texts::date_preset(label, &timezone.format_date(date)),
                deadline_callback(action),
            )
        })
    };
    let mut rows: Vec<Row> = [
        preset(
            texts::PRESET_WEEK,
            timezone.in_days(7),
            DeadlineAction::Days(7),
        ),
        preset(
            texts::PRESET_TWO_WEEKS,
            timezone.in_days(14),
            DeadlineAction::Days(14),
        ),
        preset(
            texts::PRESET_MONTH,
            timezone.in_months(1),
            DeadlineAction::Months(1),
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    rows.extend(date_input_rows());

    InlineKeyboardMarkup::new(rows)
}

fn duration_options_keyboard(
    terms: &AssignmentTerms,
    deadline: DateTime<Utc>,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let mut rows = duration_presets(terms, deadline, timezone).unwrap_or_default();
    rows.extend(date_input_rows());

    InlineKeyboardMarkup::new(rows)
}

fn duration_presets(
    terms: &AssignmentTerms,
    deadline: DateTime<Utc>,
    timezone: &BotTimezone,
) -> Option<Vec<Row>> {
    let policy = terms.preview_policy(1)?;
    let presets = [
        (
            texts::PRESET_WEEK_AFTER,
            deadline.checked_add_signed(chrono::Duration::days(7)),
        ),
        (
            texts::PRESET_TWO_WEEKS_AFTER,
            deadline.checked_add_signed(chrono::Duration::days(14)),
        ),
        (texts::PRESET_MONTH_AFTER, timezone.add_months(deadline, 1)),
    ];
    let mut rows: Vec<(u16, Row)> = presets
        .into_iter()
        .rev()
        .filter_map(|(label, at)| {
            let periods = u16::try_from(policy.periods_to_close_at(deadline, at?)).ok()?;
            let closes_at = terms.preview_policy(periods).and_then(|policy| {
                policy.validate_horizon(deadline).ok()?;
                policy.closes_at(deadline).ok()
            })?;
            let button = row(
                texts::date_preset(
                    label,
                    &texts::until(&timezone.format_until_short(closes_at)),
                ),
                FlowAction::DurationPeriods(periods).callback(),
            );
            Some((periods, button))
        })
        .collect();
    rows.dedup_by_key(|(periods, _)| *periods);
    Some(rows.into_iter().rev().map(|(_, row)| row).collect())
}

pub(crate) fn calendar_keyboard(month: NaiveDate, today: NaiveDate) -> InlineKeyboardMarkup {
    let noop = || CallbackAction::Noop.callback();
    let blank = || button("·", noop());
    let turn = |label: &str, month: NaiveDate| {
        button(label, deadline_callback(DeadlineAction::Month(month)))
    };
    let pick =
        |label: String, day: NaiveDate| button(label, deadline_callback(DeadlineAction::Pick(day)));
    let first = month.with_day(1).unwrap_or(month);
    let previous = first.checked_sub_months(chrono::Months::new(1));
    let next = first.checked_add_months(chrono::Months::new(1));

    let back = match previous {
        Some(previous) if previous >= today.with_day(1).unwrap_or(today) => turn("◀️", previous),
        _ => blank(),
    };
    let forward = next.map_or_else(blank, |next| turn("▶️", next));
    let title = format!(
        "{} {}",
        texts::MONTHS[(first.month() - 1) as usize],
        first.year()
    );

    let mut rows = vec![
        vec![back, button(title, noop()), forward],
        texts::WEEKDAYS
            .into_iter()
            .map(|day| button(day, noop()))
            .collect(),
    ];

    let mut week = vec![blank(); first.weekday().num_days_from_monday() as usize];
    let mut day = first;
    while day.month() == first.month() {
        week.push(if day < today {
            blank()
        } else if day == today {
            pick(format!("»{}", day.day()), day)
        } else {
            pick(day.day().to_string(), day)
        });
        if week.len() == 7 {
            rows.push(std::mem::take(&mut week));
        }
        let Some(tomorrow) = day.succ_opt() else {
            break;
        };
        day = tomorrow;
    }
    if !week.is_empty() {
        week.resize(7, blank());
        rows.push(week);
    }
    rows.push(row(
        texts::BUTTON_DATE_OPTIONS,
        deadline_callback(DeadlineAction::Options),
    ));

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn attempts_keyboard(step: fn(AmountChoice) -> FlowAction) -> InlineKeyboardMarkup {
    let amount = |amount: u16| {
        button(
            amount.to_string(),
            step(AmountChoice::Preset(amount)).callback(),
        )
    };

    InlineKeyboardMarkup::new(vec![
        [1, 2, 3].into_iter().map(amount).collect(),
        [5, 10].into_iter().map(amount).collect(),
        row(
            texts::BUTTON_OTHER_AMOUNT,
            step(AmountChoice::Manual).callback(),
        ),
    ])
}

pub(crate) fn grading_keyboard() -> InlineKeyboardMarkup {
    let grading = |choice| FlowAction::Grading(choice).callback();
    let preset = |max_score: u16| {
        button(
            texts::scale_button(max_score),
            grading(GradingChoice::Preset(max_score)),
        )
    };

    InlineKeyboardMarkup::new(vec![
        vec![preset(5), preset(10)],
        row(
            texts::BUTTON_OTHER_MAX_SCORE,
            grading(GradingChoice::Manual),
        ),
        row(texts::BUTTON_PASS_FAIL, grading(GradingChoice::PassFail)),
    ])
}

pub(crate) fn late_policy_keyboard() -> InlineKeyboardMarkup {
    let policy = |choice| FlowAction::LatePolicy(choice).callback();
    InlineKeyboardMarkup::new(vec![
        row(texts::BUTTON_STRICT, policy(LatePolicyChoice::Strict)),
        row(
            texts::BUTTON_LATE_WORK_SCORE,
            policy(LatePolicyChoice::Penalty(DeductedFrom::WorkScore)),
        ),
        row(
            texts::BUTTON_LATE_COURSE_TOTAL,
            policy(LatePolicyChoice::Penalty(DeductedFrom::CourseTotal)),
        ),
    ])
}

pub(crate) fn penalty_keyboard(period_days: u16) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        [1, 2]
            .into_iter()
            .map(|points| {
                button(
                    texts::penalty_button(points),
                    FlowAction::Penalty(AmountChoice::Preset(points)).callback(),
                )
            })
            .collect(),
        row(
            texts::BUTTON_OTHER_AMOUNT,
            FlowAction::Penalty(AmountChoice::Manual).callback(),
        ),
        row(
            texts::period_button(period_days),
            FlowAction::PenaltyPeriod.callback(),
        ),
    ])
}

pub(crate) fn terms_keyboard(
    terms: &AssignmentTerms,
    complete: &CompleteTerms,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let creating = matches!(terms.target, TermsTarget::Create(_));
    let mut rows = vec![
        row(
            if creating {
                texts::BUTTON_SAVE_DRAFT
            } else {
                texts::BUTTON_SAVE_TERMS
            },
            terms_callback(TermsAction::Save),
        ),
        row(
            format!(
                "{} {}",
                texts::SUBMIT_UNTIL,
                texts::until(&timezone.format_until_short(complete.deadline_at))
            ),
            terms_callback(TermsAction::Deadline),
        ),
    ];
    if let Some(policy) = complete.late_policy {
        rows.push(vec![
            button(
                format!("{} −{}", texts::PENALTY_LABEL, policy.penalty_per_period()),
                terms_callback(TermsAction::Penalty),
            ),
            button(
                texts::period_summary(policy.period_days()),
                terms_callback(TermsAction::Period),
            ),
        ]);
        rows.push(row(
            format!(
                "{} {}",
                texts::LATENESS_LABEL,
                texts::lateness_duration(policy.period_days(), policy.max_periods())
            ),
            terms_callback(TermsAction::Periods),
        ));
    }
    if terms.grading().is_scored() {
        rows.push(row(
            match terms.deducted_from {
                Some(DeductedFrom::WorkScore) => texts::BUTTON_POLICY_WORK_SCORE,
                Some(DeductedFrom::CourseTotal) => texts::BUTTON_POLICY_COURSE_TOTAL,
                None => texts::BUTTON_POLICY_STRICT,
            },
            terms_callback(TermsAction::Policy),
        ));
    }
    if terms.makeup_enabled {
        rows.push(row(
            texts::makeup_terms_line(terms.makeup, terms.grading()),
            terms_callback(TermsAction::Makeup),
        ));
    }
    rows.push(row(
        if creating {
            texts::BUTTON_CANCEL_CREATION
        } else {
            texts::BUTTON_CANCEL_TERMS
        },
        terms_callback(TermsAction::Cancel),
    ));

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn makeup_answer_keyboard(grading: Grading, hint: Option<u16>) -> InlineKeyboardMarkup {
    let answer = |label: String, choice: MakeupChoice| {
        button(label, FlowAction::MakeupAnswer(choice).callback())
    };
    let mut rows: Vec<Row> = match grading.max_score() {
        Some(max_score) if max_score <= SCORE_BUTTONS_LIMIT => (1..=max_score)
            .collect::<Vec<_>>()
            .chunks(5)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(|&credit| answer(credit.to_string(), MakeupChoice::Credit(credit)))
                    .collect()
            })
            .collect(),
        Some(_) => vec![vec![answer(
            texts::BUTTON_OTHER_AMOUNT.to_owned(),
            MakeupChoice::Manual,
        )]],
        None => vec![vec![answer(
            texts::BUTTON_OFFER_MAKEUP.to_owned(),
            MakeupChoice::PassFail,
        )]],
    };
    if let Some(credit) = hint.filter(|_| grading.is_scored()) {
        rows.push(vec![answer(
            texts::makeup_hint_button(credit),
            MakeupChoice::Credit(credit),
        )]);
    }
    rows.push(vec![answer(
        texts::BUTTON_NO_MAKEUP.to_owned(),
        MakeupChoice::NotOffered,
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn makeup_attempts_keyboard() -> InlineKeyboardMarkup {
    let amount = |amount: u16| {
        button(
            amount.to_string(),
            FlowAction::MakeupAttempts(AmountChoice::Preset(amount)).callback(),
        )
    };

    InlineKeyboardMarkup::new(vec![
        [1, 2, 3, 5].into_iter().map(amount).collect(),
        row(
            texts::BUTTON_OTHER_AMOUNT,
            FlowAction::MakeupAttempts(AmountChoice::Manual).callback(),
        ),
    ])
}

fn makeup_summary_keyboard(form: &MakeupForm, timezone: &BotTimezone) -> InlineKeyboardMarkup {
    let mut rows = vec![row(
        texts::BUTTON_SAVE_MAKEUP,
        makeup_form_callback(MakeupFormAction::Save),
    )];
    if let (false, Some(until), Some(attempts)) = (form.disabling, form.until, form.attempts) {
        rows.push(row(
            texts::makeup_date_button(&texts::until(&timezone.format_until_short(until))),
            makeup_form_callback(MakeupFormAction::Date),
        ));
        rows.push(row(
            texts::makeup_attempts_button(attempts),
            makeup_form_callback(MakeupFormAction::Attempts),
        ));
    }
    if form.active {
        rows.push(row(
            if form.disabling {
                texts::BUTTON_KEEP_MAKEUP
            } else {
                texts::BUTTON_DISABLE_MAKEUP
            },
            makeup_form_callback(MakeupFormAction::Disable),
        ));
    }
    rows.push(row(
        texts::BUTTON_CANCEL_MAKEUP,
        makeup_form_callback(MakeupFormAction::Cancel),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(super) fn makeup_form_keyboard(
    form: &MakeupForm,
    timezone: &BotTimezone,
    now: DateTime<Utc>,
) -> Option<InlineKeyboardMarkup> {
    let mut keyboard = match form.step {
        MakeupStep::Date => match form.mode {
            InputMode::Calendar(month) => {
                calendar_keyboard(month, timezone.first_date_ending_after(now))
            }
            InputMode::Buttons | InputMode::Manual => deadline_options_keyboard(timezone),
        },
        MakeupStep::Attempts => makeup_attempts_keyboard(),
        MakeupStep::Scale(_) => makeup_answer_keyboard(form.current_scale()?.grading, None),
        MakeupStep::Summary => makeup_summary_keyboard(form, timezone),
    };
    if form.can_go_back() {
        keyboard.inline_keyboard.push(row(
            texts::BUTTON_BACK,
            makeup_form_callback(MakeupFormAction::Back),
        ));
    }
    Some(keyboard)
}

pub(crate) fn terms_exit_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(texts::BUTTON_STAY, terms_callback(TermsAction::Stay)),
        row(texts::BUTTON_EXIT, terms_callback(TermsAction::Exit)),
    ])
}

pub(super) fn terms_form_keyboard(
    form: &TermsForm,
    timezone: &BotTimezone,
    now: DateTime<Utc>,
) -> Option<InlineKeyboardMarkup> {
    let terms = &form.terms;
    let calendar = |month, after| calendar_keyboard(month, timezone.first_date_ending_after(after));
    let mut keyboard = match form.step {
        TermsStep::Makeup => makeup_answer_keyboard(terms.grading(), terms.makeup_hint),
        TermsStep::Policy => late_policy_keyboard(),
        TermsStep::Deadline => match (form.mode, &terms.target) {
            (InputMode::Calendar(month), TermsTarget::Edit { created_at, .. }) => {
                calendar(month, *created_at)
            }
            (InputMode::Calendar(month), TermsTarget::Create(_)) => calendar(month, now),
            _ => deadline_options_keyboard(timezone),
        },
        TermsStep::DurationDate => {
            let deadline = terms.deadline_at?;
            match form.mode {
                InputMode::Calendar(month) => calendar(month, deadline),
                _ => duration_options_keyboard(terms, deadline, timezone),
            }
        }
        TermsStep::Penalty => penalty_keyboard(terms.period_days),
        TermsStep::Period => InlineKeyboardMarkup::default(),
        TermsStep::Periods => {
            let mut rows = Vec::new();
            if let Some(deadline) = terms.deadline_at {
                for periods in [1, 2, 4] {
                    let Some(closes_at) = terms.preview_policy(periods).and_then(|policy| {
                        policy.validate_horizon(deadline).ok()?;
                        policy.closes_at(deadline).ok()
                    }) else {
                        continue;
                    };
                    rows.push(row(
                        texts::periods_option(
                            &texts::lateness_duration(terms.period_days, periods),
                            &timezone.format_until_short(closes_at),
                        ),
                        FlowAction::Periods(AmountChoice::Preset(periods)).callback(),
                    ));
                }
            }
            rows.push(vec![
                button(
                    texts::BUTTON_OTHER_PERIODS,
                    FlowAction::Periods(AmountChoice::Manual).callback(),
                ),
                button(
                    texts::BUTTON_CALENDAR,
                    terms_callback(TermsAction::DurationDate),
                ),
            ]);
            InlineKeyboardMarkup::new(rows)
        }
        TermsStep::DurationConfirm { .. } => InlineKeyboardMarkup::new(vec![row(
            texts::BUTTON_APPLY_PERIODS,
            terms_callback(TermsAction::DurationApply),
        )]),
        TermsStep::Summary => terms_keyboard(terms, &terms.complete()?, timezone),
    };
    if form.mode != InputMode::Manual && form.can_go_back() {
        keyboard
            .inline_keyboard
            .push(row(texts::BUTTON_BACK, terms_callback(TermsAction::Back)));
    }
    Some(keyboard)
}

pub(crate) const SCORE_BUTTONS_LIMIT: u16 = 10;

/// Для длинной шкалы нужен ручной ввод: кнопки на весь диапазон `0..=100` не помещаются.
pub(crate) fn review_score_keyboard(
    submission_id: SubmissionId,
    max_score: u16,
) -> InlineKeyboardMarkup {
    let choose = |choice| {
        FlowAction::ReviewScore {
            submission_id,
            choice,
        }
        .callback()
    };
    let score = |score: u16| button(score.to_string(), choose(AmountChoice::Preset(score)));

    let mut rows: Vec<Row> = (0..=max_score)
        .collect::<Vec<_>>()
        .chunks(6)
        .map(|chunk| chunk.iter().copied().map(score).collect())
        .collect();
    rows.push(row(
        texts::BUTTON_OTHER_AMOUNT,
        choose(AmountChoice::Manual),
    ));

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn assignment_files_keyboard(
    assignment: &Assignment,
    files: &[StoredFileMetadata],
    can_add: bool,
    creation_flow: bool,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            vec![
                button(
                    button_label(&texts::file_button(file.filename().as_str())),
                    ScreenAction::OpenFile(index).callback(),
                ),
                button("🗑", ScreenAction::DeleteFile(index).callback()),
            ]
        })
        .collect();
    if can_add {
        rows.push(row(
            texts::BUTTON_ADD_FILE,
            ScreenAction::AddFile.callback(),
        ));
    }
    rows.push(if creation_flow {
        row(
            texts::BUTTON_FINISH_CREATION,
            ScreenAction::FinishFileCreation.callback(),
        )
    } else {
        row(
            texts::BUTTON_BACK_TO_ASSIGNMENT,
            ReplayableAction::ManageAssignment(assignment.id()).callback(),
        )
    });

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn confirm_file_deletion_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        button(
            texts::BUTTON_DELETE,
            FlowAction::ConfirmFileDeletion.callback(),
        ),
        button(
            texts::BUTTON_CANCEL,
            FlowAction::CancelFileDeletion.callback(),
        ),
    ]])
}

pub(crate) fn course_invite_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(
            texts::BUTTON_ROTATE_INVITE,
            ScreenAction::RotateCourseInvite.callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSE,
            ReplayableAction::Page(PageTarget::Course(course_id), 0).callback(),
        ),
    ])
}

pub(crate) fn confirm_invite_rotation_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        button(
            texts::BUTTON_CONFIRM_ROTATE,
            FlowAction::ConfirmInviteRotation.callback(),
        ),
        button(
            texts::BUTTON_CANCEL,
            FlowAction::CancelInviteRotation.callback(),
        ),
    ]])
}

pub(crate) fn registration_groups_keyboard(groups: &[StudentGroup]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(groups.iter().map(|group| {
        row(
            button_label(group.code().as_str()),
            FlowAction::RegistrationGroup(group.id()).callback(),
        )
    }))
}

pub(crate) fn assignment_groups_keyboard(
    groups: &[StudentGroup],
    selected: &BTreeSet<StudentGroupId>,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .iter()
        .map(|group| {
            row(
                button_label(&texts::group_choice_button(
                    group.code().as_str(),
                    selected.contains(&group.id()),
                )),
                FlowAction::AssignmentGroup(group.id()).callback(),
            )
        })
        .collect();
    rows.push(row(
        texts::BUTTON_GROUPS_DONE,
        FlowAction::AssignmentGroupsDone.callback(),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn roster_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(
            texts::BUTTON_GRADEBOOK,
            ReplayableAction::GradebookGroups(course_id).callback(),
        ),
        row(
            texts::BUTTON_COURSE_INVITE,
            ReplayableAction::CourseInvite(course_id).callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSE,
            ReplayableAction::Page(PageTarget::Course(course_id), 0).callback(),
        ),
    ])
}

pub(crate) fn gradebook_groups_keyboard(
    course_id: TeachingCourseId,
    groups: &[StudentGroup],
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = groups
        .iter()
        .map(|group| {
            row(
                button_label(&texts::group_button(group.code().as_str())),
                ReplayableAction::Page(PageTarget::Gradebook(course_id, group.id()), 0).callback(),
            )
        })
        .collect();
    rows.push(row(
        texts::BUTTON_BACK_TO_COURSE,
        ReplayableAction::Page(PageTarget::Course(course_id), 0).callback(),
    ));
    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn gradebook_students_keyboard(
    course_id: TeachingCourseId,
    students: &[CourseStudent],
) -> InlineKeyboardMarkup {
    students_keyboard(
        students
            .iter()
            .map(|student| (student.student_id, student.display_name.as_str())),
        row(
            texts::BUTTON_BACK_TO_COURSE,
            ReplayableAction::Page(PageTarget::Course(course_id), 0).callback(),
        ),
    )
}

pub(crate) fn assignment_students_keyboard(
    assignment_id: AssignmentId,
    students: &[AssignmentStudentSummary],
    with_groups: bool,
) -> InlineKeyboardMarkup {
    let names: Vec<String> = students
        .iter()
        .map(|item| {
            let name = item.student.display_name().as_str();
            if with_groups {
                texts::group_student(&item.group_code, name)
            } else {
                name.to_owned()
            }
        })
        .collect();
    students_keyboard(
        students
            .iter()
            .zip(&names)
            .map(|(item, name)| (item.student.id(), name.as_str())),
        row(
            texts::BUTTON_BACK_TO_ASSIGNMENT,
            ReplayableAction::ManageAssignment(assignment_id).callback(),
        ),
    )
}

fn students_keyboard<'a>(
    students: impl Iterator<Item = (UserId, &'a str)>,
    back: Row,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Row> = students
        .map(|(student_id, name)| {
            row(
                button_label(&texts::student_button(name)),
                FlowAction::SelectStudent(student_id).callback(),
            )
        })
        .collect();
    rows.push(back);

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn paginated_keyboard(
    mut keyboard: InlineKeyboardMarkup,
    target: PageTarget,
    page: u32,
    next: Option<u32>,
) -> InlineKeyboardMarkup {
    let previous = page
        .checked_sub(1)
        .map(|previous| button(texts::BUTTON_PREVIOUS_PAGE, target.callback(previous)));
    let next = next.map(|next| button(texts::BUTTON_NEXT_PAGE, target.callback(next)));
    let buttons: Row = previous.into_iter().chain(next).collect();
    if !buttons.is_empty() {
        keyboard.inline_keyboard.push(buttons);
    }
    keyboard
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use chrono_tz::Tz;

    use super::super::render::{penalty_schedule, terms_form_view};
    use super::*;
    use crate::bot::assignment_form::{AssignmentText, DraftAssignment, IssuedGroups};
    use crate::bot::texts::deadline_left;
    use crate::domain::{Grading, LatePolicy, MaxScore};
    use crate::test_support::t0;

    fn scored_terms() -> AssignmentTerms {
        let mut draft = DraftAssignment::new(
            AssignmentText {
                course_id: TeachingCourseId::new(),
                title: "Task".into(),
                body: "Body".into(),
            },
            IssuedGroups::Single(StudentGroupId::new()),
            2,
        );
        draft.grading = Grading::scored(10).unwrap();
        AssignmentTerms {
            deadline_at: Some(crate::test_support::t0() + chrono::Duration::days(7)),
            deducted_from: Some(DeductedFrom::WorkScore),
            period_days: 1,
            penalty_per_period: Some(1),
            max_periods: Some(100),
            ..AssignmentTerms::creating(draft, crate::app::MakeupQuestion::default())
        }
    }

    fn preset_periods(keyboard: &InlineKeyboardMarkup) -> Vec<u16> {
        use crate::bot::callbacks::{CallbackAction, FlowAction};
        keyboard
            .inline_keyboard
            .iter()
            .flatten()
            .filter_map(|button| {
                let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = &button.kind
                else {
                    panic!("expected a callback");
                };
                match crate::bot::callbacks::parse(data).expect("routable duration button") {
                    CallbackAction::Flow(FlowAction::DurationPeriods(periods)) => Some(periods),
                    _ => None,
                }
            })
            .collect()
    }

    #[test]
    fn duration_presets_count_whole_periods_from_the_deadline_moment() {
        let timezone = timezone();
        for deadline in ["20.08.2027", "20.08.2027 23:59"] {
            let mut terms = scored_terms();
            terms.deadline_at = timezone.parse_deadline(deadline);
            terms.period_days = 7;
            let deadline = terms.deadline_at.unwrap();
            let keyboard = duration_options_keyboard(&terms, deadline, &timezone);
            assert_eq!(preset_periods(&keyboard), [1, 2, 5], "{deadline}");
        }
    }

    #[test]
    fn presets_rounding_up_to_the_same_period_leave_one_button_named_by_the_longest() {
        let timezone = timezone();
        let mut terms = scored_terms();
        terms.deadline_at = timezone.parse_deadline("20.08.2027");
        terms.period_days = 14;
        let deadline = terms.deadline_at.unwrap();

        let keyboard = duration_options_keyboard(&terms, deadline, &timezone);

        assert_eq!(preset_periods(&keyboard), [1, 3]);
        assert_eq!(
            keyboard.inline_keyboard[0][0].text,
            texts::date_preset(
                texts::PRESET_TWO_WEEKS_AFTER,
                &texts::until(&timezone.format_until_short(deadline + chrono::Duration::days(14)))
            )
        );
    }

    #[test]
    fn a_duration_preset_names_the_moment_submissions_close() {
        let timezone = timezone();
        let mut terms = scored_terms();
        terms.deadline_at = timezone.parse_deadline("20.08.2027 23:59");
        terms.period_days = 7;
        let deadline = terms.deadline_at.unwrap();
        let week = deadline + chrono::Duration::days(7);

        let keyboard = duration_options_keyboard(&terms, deadline, &timezone);

        assert_eq!(
            keyboard.inline_keyboard[0][0].text,
            texts::date_preset(
                texts::PRESET_WEEK_AFTER,
                &texts::until(&timezone.format_until_short(week))
            )
        );
    }

    #[test]
    fn edit_calendar_hides_the_utc_day_before_local_creation() {
        let timezone = timezone();
        let created_at = "2026-08-20T15:00:00Z".parse().unwrap();
        let mut terms = scored_terms();
        terms.target = TermsTarget::Edit {
            assignment_id: AssignmentId::new(),
            grading: terms.grading(),
            revision: 0,
            created_at,
        };
        let mut form = TermsForm::new(terms, TermsStep::Deadline);
        form.mode = InputMode::Calendar(timezone.local_date(created_at));
        let (_, keyboard) = terms_form_view(&form, &timezone, t0()).unwrap();
        let json = serde_json::to_string(&keyboard).unwrap();
        assert!(!json.contains(&pick(2026, 8, 20)));
        assert!(json.contains(&pick(2026, 8, 21)));
    }

    #[test]
    fn a_hundred_periods_still_produce_a_short_schedule() {
        let form = TermsForm::new(scored_terms(), TermsStep::Summary);
        let (text, keyboard) = terms_form_view(&form, &timezone(), t0()).unwrap();
        assert!(text.contains(texts::SCHEDULE_HEADER), "{text}");
        assert!(text.chars().count() < 1000, "{text}");
        assert_eq!(keyboard.inline_keyboard.last().unwrap().len(), 1);
        assert_eq!(
            keyboard.inline_keyboard.last().unwrap()[0].text,
            texts::BUTTON_CANCEL_CREATION
        );
    }

    #[test]
    fn the_schedule_names_every_penalty_step_and_the_closing_date() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(7, 1, 3, DeductedFrom::WorkScore).unwrap();
        let max_score = MaxScore::new(10).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);

        assert_eq!(
            schedule.lines().collect::<Vec<_>>(),
            [
                texts::SCHEDULE_HEADER,
                &texts::schedule_free_row("20.08.2026 23:59", 10),
                &texts::schedule_penalty_row("27.08.2026 23:59", 1, 9, 10),
                &texts::schedule_penalty_row("03.09.2026 23:59", 2, 8, 10),
                &texts::schedule_penalty_row("10.09.2026 23:59", 3, 7, 10),
                &texts::schedule_closed("10.09.2026 23:59"),
            ]
        );
    }

    #[test]
    fn the_schedule_stops_at_the_period_that_zeroes_the_score() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(1, 5, 40, DeductedFrom::WorkScore).unwrap();
        let max_score = MaxScore::new(10).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);
        let lines: Vec<_> = schedule.lines().collect();

        assert_eq!(lines.len(), 5, "{schedule}");
        assert_eq!(
            lines[3],
            texts::schedule_penalty_row("22.08.2026 23:59", 10, 0, 10)
        );
        assert_eq!(lines[4], texts::schedule_closed("29.09.2026 23:59"));
    }

    #[test]
    fn the_zeroing_period_names_the_whole_penalty_the_review_will_store() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(1, 6, 5, DeductedFrom::WorkScore).unwrap();
        let max_score = MaxScore::new(10).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);
        let lines: Vec<_> = schedule.lines().collect();

        assert_eq!(
            lines[3],
            texts::schedule_penalty_row("22.08.2026 23:59", 12, 0, 10)
        );
    }

    #[test]
    fn a_long_schedule_collapses_its_middle_into_one_line() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(1, 1, 40, DeductedFrom::WorkScore).unwrap();
        let max_score = MaxScore::new(100).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);
        let lines: Vec<_> = schedule.lines().collect();

        assert_eq!(lines.len(), 10, "{schedule}");
        assert_eq!(lines[7], texts::schedule_skipped(34));
        assert_eq!(
            lines[8],
            texts::schedule_penalty_row("29.09.2026 23:59", 40, 60, 100)
        );
        assert_eq!(
            lines.last().unwrap(),
            &texts::schedule_closed("29.09.2026 23:59")
        );
    }

    #[test]
    fn a_schedule_to_the_course_total_names_the_penalty_without_a_possible_score() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(7, 2, 2, DeductedFrom::CourseTotal).unwrap();
        let max_score = MaxScore::new(10).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);

        assert_eq!(
            schedule.lines().collect::<Vec<_>>(),
            [
                texts::SCHEDULE_TOTAL_HEADER,
                &texts::schedule_free_total_row("20.08.2026 23:59"),
                &texts::schedule_total_row("27.08.2026 23:59", 2),
                &texts::schedule_total_row("03.09.2026 23:59", 4),
                &texts::schedule_closed("03.09.2026 23:59"),
            ]
        );
        assert!(!schedule.contains("из 10"), "{schedule}");
    }

    #[test]
    fn a_schedule_to_the_course_total_runs_to_the_period_limit() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(1, 5, 40, DeductedFrom::CourseTotal).unwrap();
        let max_score = MaxScore::new(10).unwrap();

        let schedule = schedule(deadline, policy, max_score, &timezone);
        let lines: Vec<_> = schedule.lines().collect();

        assert_eq!(lines.len(), 10, "{schedule}");
        assert_eq!(lines[7], texts::schedule_skipped(34));
        assert_eq!(lines[8], texts::schedule_total_row("29.09.2026 23:59", 100));
        assert_eq!(lines[9], texts::schedule_closed("29.09.2026 23:59"));
    }

    #[test]
    fn calendar_return_and_previous_step_have_distinct_labels_and_actions() {
        let mut form = TermsForm::new(scored_terms(), TermsStep::Deadline);
        form.mode = InputMode::Calendar(timezone().today());
        let (_, keyboard) = terms_form_view(&form, &timezone(), t0()).unwrap();
        let json = serde_json::to_value(keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows[rows.len() - 2][0]["text"], texts::BUTTON_DATE_OPTIONS);
        assert_eq!(
            rows[rows.len() - 2][0]["callback_data"],
            deadline_callback(DeadlineAction::Options)
        );
        assert_eq!(rows.last().unwrap()[0]["text"], texts::BUTTON_BACK);
        assert_eq!(
            rows.last().unwrap()[0]["callback_data"],
            terms_callback(TermsAction::Back)
        );
    }

    #[test]
    fn the_makeup_form_offers_a_step_back_only_where_one_exists() {
        let back = makeup_form_callback(MakeupFormAction::Back);
        let last_action = |form: &MakeupForm| {
            let keyboard = makeup_form_keyboard(form, &timezone(), t0()).unwrap();
            let json = serde_json::to_value(keyboard).unwrap();
            json["inline_keyboard"].as_array().unwrap().last().unwrap()[0]["callback_data"].clone()
        };
        let mut form = MakeupForm::new(TeachingCourseId::new(), None, Vec::new(), t0());
        assert_ne!(last_action(&form), back);

        form.apply_date(t0() + chrono::Duration::days(30), t0())
            .unwrap();
        assert_eq!(last_action(&form), back);
        form.apply_attempts(3).unwrap();
        assert_eq!(form.step, MakeupStep::Summary);
        assert_ne!(last_action(&form), back);

        assert!(form.edit(MakeupFormAction::Date));
        assert_eq!(last_action(&form), back);
    }

    fn timezone() -> BotTimezone {
        BotTimezone::new(Tz::Asia__Vladivostok)
    }

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn pick(year: i32, month: u32, day: u32) -> String {
        deadline_callback(DeadlineAction::Pick(date(year, month, day)))
    }

    fn schedule(
        deadline: DateTime<Utc>,
        policy: LatePolicy,
        max_score: MaxScore,
        timezone: &BotTimezone,
    ) -> String {
        let closes_at = policy.closes_at(deadline).unwrap();
        penalty_schedule(deadline, policy, closes_at, max_score, timezone)
    }

    #[test]
    fn a_form_that_cannot_be_summarised_has_no_view_instead_of_a_panic() {
        let timezone = timezone();
        let mut terms = scored_terms();
        terms.deadline_at = Some(DateTime::<Utc>::MAX_UTC - chrono::Duration::days(1));
        terms.max_periods = Some(4);
        assert!(terms.validate().is_err());

        for step in [
            TermsStep::Summary,
            TermsStep::DurationConfirm {
                selected: DateTime::<Utc>::MAX_UTC,
                periods: 4,
            },
        ] {
            let form = TermsForm::new(terms.clone(), step);
            assert!(
                terms_form_view(&form, &timezone, t0()).is_none(),
                "{step:?}"
            );
        }

        let mut terms = scored_terms();
        terms.deadline_at = None;
        let form = TermsForm::new(terms, TermsStep::DurationDate);
        assert!(terms_form_view(&form, &timezone, t0()).is_none());
    }

    #[test]
    fn button_labels_are_truncated_only_when_needed() {
        assert_eq!(button_label("Мои задания"), "Мои задания");
        assert_eq!(button_label(&"я".repeat(61)).chars().count(), 60);
        assert!(button_label(&"я".repeat(61)).ends_with("..."));
    }

    #[test]
    fn bottom_menu_buttons_route_back_to_their_screens() {
        assert_eq!(
            menu_action(texts::BUTTON_ASSIGNMENTS),
            Some(MenuAction::Assignments)
        );
        assert_eq!(menu_action(texts::BUTTON_QUEUE), Some(MenuAction::Queue));
        assert_eq!(menu_action(texts::BUTTON_REVIEW), Some(MenuAction::Review));
        assert_eq!(
            menu_action(texts::BUTTON_SANDBOX),
            Some(MenuAction::Sandbox)
        );
        assert_eq!(menu_action("ссылка на pull request"), None);
    }

    #[test]
    fn calendar_hides_past_days_and_the_way_back_to_past_months() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 13).unwrap();
        let json = serde_json::to_value(calendar_keyboard(today, today)).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        let callbacks: Vec<_> = rows
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .map(|button| button["callback_data"].as_str().unwrap())
            .collect();

        let month = |month| deadline_callback(DeadlineAction::Month(date(2026, month, 1)));
        assert!(!callbacks.contains(&month(7).as_str()));
        assert!(callbacks.contains(&month(9).as_str()));
        assert!(!callbacks.contains(&pick(2026, 8, 12).as_str()));
        assert!(callbacks.contains(&pick(2026, 8, 13).as_str()));
        assert!(callbacks.contains(&pick(2026, 8, 31).as_str()));
        assert!(rows.iter().all(|row| row.as_array().unwrap().len() <= 7));
    }

    #[test]
    fn deadline_presets_offer_a_week_two_weeks_and_a_month() {
        let json = serde_json::to_value(deadline_options_keyboard(&timezone())).unwrap();
        let callbacks: Vec<_> = json["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[0]["callback_data"].as_str().unwrap().to_owned())
            .collect();

        assert_eq!(
            callbacks,
            [
                DeadlineAction::Days(7),
                DeadlineAction::Days(14),
                DeadlineAction::Months(1),
                DeadlineAction::Calendar,
                DeadlineAction::Manual,
            ]
            .map(deadline_callback)
        );
    }

    #[test]
    fn passed_deadlines_are_not_reported_as_time_left() {
        let past: DateTime<Utc> = Utc::now() - chrono::Duration::hours(1);

        assert_eq!(deadline_left(past, Utc::now()), texts::DEADLINE_PASSED);
    }

    #[test]
    fn the_makeup_question_offers_the_common_credit_as_a_separate_button() {
        let ten = Grading::scored(10).unwrap();
        let buttons = |keyboard: InlineKeyboardMarkup| -> Vec<(String, String)> {
            let json = serde_json::to_value(keyboard).unwrap();
            json["inline_keyboard"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|row| row.as_array().unwrap().clone())
                .map(|button| {
                    (
                        button["text"].as_str().unwrap().to_owned(),
                        button["callback_data"].as_str().unwrap().to_owned(),
                    )
                })
                .collect()
        };
        let answer = |choice| FlowAction::MakeupAnswer(choice).callback();
        let six = answer(MakeupChoice::Credit(6));

        let with_hint = buttons(makeup_answer_keyboard(ten, Some(6)));
        assert!(with_hint.contains(&(texts::makeup_hint_button(6), six.clone())));
        assert!(with_hint.contains(&("6".to_owned(), six.clone())));
        assert!(
            !buttons(makeup_answer_keyboard(ten, None))
                .contains(&(texts::makeup_hint_button(6), six))
        );

        let long = buttons(makeup_answer_keyboard(
            Grading::scored(20).unwrap(),
            Some(12),
        ));
        assert!(long.contains(&(
            texts::makeup_hint_button(12),
            answer(MakeupChoice::Credit(12))
        )));
        assert!(long.contains(&(
            texts::BUTTON_OTHER_AMOUNT.to_owned(),
            answer(MakeupChoice::Manual)
        )));
        assert!(
            buttons(makeup_answer_keyboard(Grading::PassFail, Some(6)))
                .iter()
                .all(|(_, data)| !data.ends_with(":6"))
        );
    }

    #[test]
    fn the_review_hub_offers_the_queue_only_when_it_has_work() {
        let callbacks = |keyboard: InlineKeyboardMarkup| -> Vec<String> {
            keyboard
                .inline_keyboard
                .into_iter()
                .flatten()
                .filter_map(|button| match button.kind {
                    teloxide::types::InlineKeyboardButtonKind::CallbackData(data) => Some(data),
                    _ => None,
                })
                .collect()
        };

        assert_eq!(
            callbacks(review_hub_keyboard(0, 2)),
            vec![ReplayableAction::NextMakeupReview.callback()]
        );
        assert_eq!(
            callbacks(review_hub_keyboard(3, 2)),
            vec![
                ReplayableAction::NextReview.callback(),
                ReplayableAction::NextMakeupReview.callback(),
            ]
        );
    }
}
