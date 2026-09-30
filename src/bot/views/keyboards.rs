use chrono::{DateTime, Datelike, NaiveDate, Utc};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup};

use super::render::button_label;
use crate::app::{AssignmentStudentSummary, CourseStudent, TeachingCourseSummary};
use crate::bot::assignment_form::{
    AssignmentTerms, CompleteTerms, InputMode, TermsForm, TermsStep, TermsTarget,
};
use crate::bot::callbacks::{
    CALENDAR_MONTH_PREFIX, CALENDAR_NOOP, CALENDAR_PICK_PREFIX, DEADLINE_CALENDAR,
    DEADLINE_DAYS_PREFIX, DEADLINE_MANUAL, DEADLINE_MONTHS_PREFIX, DEADLINE_OPTIONS,
    FILE_CANCEL_DELETE, FILE_CONFIRM_DELETE, GRADING_PREFIX, INVITE_CANCEL_ROTATE,
    INVITE_CONFIRM_ROTATE, InformationalAction, LATE_PERIODS_PREFIX, LATE_POLICY_PREFIX,
    PENALTY_PERIOD, PENALTY_PREFIX, PageTarget, REVIEW_ACCEPT_PREFIX, REVIEW_REJECT_PREFIX,
    REVIEW_SCORE_PREFIX, ReplayableAction, STUDENT_PREFIX, ScreenAction, TERMS_BACK, TERMS_CANCEL,
    TERMS_DEADLINE, TERMS_DURATION_APPLY, TERMS_DURATION_DATE, TERMS_EXIT, TERMS_PENALTY,
    TERMS_PERIOD, TERMS_PERIODS, TERMS_POLICY, TERMS_SAVE, TERMS_STAY,
};
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;
use crate::domain::{
    Assignment, AssignmentId, AssignmentStatus, Role, StoredFileMetadata, SubmissionId,
    TeachingCourseId, UserId,
};

type Row = Vec<InlineKeyboardButton>;

fn button(label: impl Into<String>, data: impl Into<String>) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(label, data)
}

fn row(label: impl Into<String>, data: impl Into<String>) -> Row {
    vec![button(label, data)]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MenuAction {
    Assignments,
    Archive,
    Queue,
    Review,
    Courses,
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
        vec![KeyboardButton::new(texts::BUTTON_HELP)],
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
        row(review_label, ReplayableAction::NextReview.callback()),
        row(
            texts::BUTTON_COURSES,
            ReplayableAction::Page(PageTarget::Courses, 0).callback(),
        ),
        row(texts::BUTTON_PROFILE, ReplayableAction::Profile.callback()),
    ])
}

pub(crate) fn review_keyboard(
    submission_id: SubmissionId,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            button(
                texts::BUTTON_ACCEPT,
                format!("{REVIEW_ACCEPT_PREFIX}{}", submission_id.as_uuid()),
            ),
            button(
                texts::BUTTON_REJECT,
                format!("{REVIEW_REJECT_PREFIX}{}", submission_id.as_uuid()),
            ),
        ],
        vec![student_contact_button(course_id, student_id)],
    ])
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
    student_id: UserId,
) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![student_contact_button(course_id, student_id)],
        row(
            texts::BUTTON_BACK_TO_STUDENTS,
            PageTarget::Gradebook(course_id).callback(0),
        ),
    ])
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

pub(crate) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(courses.iter().map(|course| {
        row(
            button_label(&texts::course_button(
                course.subject.name().as_str(),
                course.group.code().as_str(),
            )),
            ReplayableAction::Course(course.course.id()).callback(),
        )
    }))
}

pub(crate) fn course_assignments_keyboard(
    course_id: TeachingCourseId,
    assignments: &[Assignment],
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
            ReplayableAction::Gradebook(course_id).callback(),
        ),
        row(
            texts::BUTTON_GROUP_INVITE,
            ReplayableAction::GroupInvite(course_id).callback(),
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
            ReplayableAction::GrantAttempts(assignment_id).callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSE_ASSIGNMENTS,
            ReplayableAction::Course(assignment.teaching_course_id()).callback(),
        ),
    ]);

    InlineKeyboardMarkup::new(rows)
}

fn date_input_rows() -> [Row; 2] {
    [
        row(texts::BUTTON_CALENDAR, DEADLINE_CALENDAR),
        row(texts::BUTTON_MANUAL, DEADLINE_MANUAL),
    ]
}

pub(crate) fn deadline_options_keyboard(timezone: &BotTimezone) -> InlineKeyboardMarkup {
    let preset = |label: &str, date: Option<NaiveDate>, data: String| {
        date.map(|date| row(texts::date_preset(label, &timezone.format_date(date)), data))
    };
    let mut rows: Vec<Row> = [
        preset(
            texts::PRESET_WEEK,
            timezone.in_days(7),
            format!("{DEADLINE_DAYS_PREFIX}7"),
        ),
        preset(
            texts::PRESET_TWO_WEEKS,
            timezone.in_days(14),
            format!("{DEADLINE_DAYS_PREFIX}14"),
        ),
        preset(
            texts::PRESET_MONTH,
            timezone.in_months(1),
            format!("{DEADLINE_MONTHS_PREFIX}1"),
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
    let base = timezone.local_date(deadline - chrono::Duration::nanoseconds(1));
    let mut rows = Vec::new();
    for (label, date) in [
        (
            texts::PRESET_WEEK_AFTER,
            base.checked_add_days(chrono::Days::new(7)),
        ),
        (
            texts::PRESET_TWO_WEEKS_AFTER,
            base.checked_add_days(chrono::Days::new(14)),
        ),
        (
            texts::PRESET_MONTH_AFTER,
            base.checked_add_months(chrono::Months::new(1)),
        ),
    ] {
        let Some(date) = date else { continue };
        let Some(at) = timezone.end_of(date) else {
            continue;
        };
        let Ok(policy) = terms.preview_policy(1) else {
            continue;
        };
        let Ok(periods) = u16::try_from(policy.periods_to_close_at(deadline, at)) else {
            continue;
        };
        if terms
            .preview_policy(periods)
            .and_then(|policy| policy.validate_horizon(deadline))
            .is_err()
        {
            continue;
        }
        rows.push(row(
            texts::date_preset(label, &timezone.format_date(date)),
            format!("{CALENDAR_PICK_PREFIX}{date}"),
        ));
    }
    rows.extend(date_input_rows());

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn calendar_keyboard(month: NaiveDate, today: NaiveDate) -> InlineKeyboardMarkup {
    let blank = || button("·", CALENDAR_NOOP);
    let turn = |label: &str, month: NaiveDate| {
        button(
            label,
            format!("{CALENDAR_MONTH_PREFIX}{}", month_key(month)),
        )
    };
    let pick = |label: String, day: NaiveDate| {
        button(
            label,
            format!("{CALENDAR_PICK_PREFIX}{}", day.format("%Y-%m-%d")),
        )
    };
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
        vec![back, button(title, CALENDAR_NOOP), forward],
        texts::WEEKDAYS
            .into_iter()
            .map(|day| button(day, CALENDAR_NOOP))
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
    rows.push(row(texts::BUTTON_DATE_OPTIONS, DEADLINE_OPTIONS));

    InlineKeyboardMarkup::new(rows)
}

fn month_key(date: NaiveDate) -> String {
    date.format("%Y-%m").to_string()
}

pub(crate) fn attempts_keyboard(prefix: &str) -> InlineKeyboardMarkup {
    let amount = |amount: u16| button(amount.to_string(), format!("{prefix}{amount}"));

    InlineKeyboardMarkup::new(vec![
        [1, 2, 3].into_iter().map(amount).collect(),
        [5, 10].into_iter().map(amount).collect(),
        row(texts::BUTTON_OTHER_AMOUNT, format!("{prefix}manual")),
    ])
}

pub(crate) fn grading_keyboard() -> InlineKeyboardMarkup {
    let preset = |max_score: u16| {
        button(
            texts::scale_button(max_score),
            format!("{GRADING_PREFIX}{max_score}"),
        )
    };

    InlineKeyboardMarkup::new(vec![
        vec![preset(5), preset(10)],
        row(
            texts::BUTTON_OTHER_MAX_SCORE,
            format!("{GRADING_PREFIX}manual"),
        ),
        row(texts::BUTTON_PASS_FAIL, format!("{GRADING_PREFIX}passfail")),
    ])
}

pub(crate) fn late_policy_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(texts::BUTTON_STRICT, format!("{LATE_POLICY_PREFIX}none")),
        row(texts::BUTTON_LATE, format!("{LATE_POLICY_PREFIX}penalty")),
    ])
}

pub(crate) fn penalty_keyboard(period_days: u16) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        [1, 2]
            .into_iter()
            .map(|points| {
                button(
                    texts::penalty_button(points),
                    format!("{PENALTY_PREFIX}{points}"),
                )
            })
            .collect(),
        row(
            texts::BUTTON_OTHER_NUMBER,
            format!("{PENALTY_PREFIX}manual"),
        ),
        row(texts::period_button(period_days), PENALTY_PERIOD),
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
            TERMS_SAVE,
        ),
        row(
            format!(
                "{} {}",
                texts::SUBMIT_UNTIL,
                texts::until(&timezone.format_until_short(complete.deadline_at))
            ),
            TERMS_DEADLINE,
        ),
    ];
    if let Some(policy) = complete.late_policy {
        rows.push(vec![
            button(
                format!("{} −{}", texts::PENALTY_LABEL, policy.penalty()),
                TERMS_PENALTY,
            ),
            button(texts::period_summary(policy.period_days()), TERMS_PERIOD),
        ]);
        rows.push(row(
            format!(
                "{} {}",
                texts::LATENESS_LABEL,
                texts::lateness_duration(policy.period_days(), policy.max_periods())
            ),
            TERMS_PERIODS,
        ));
    }
    if terms.grading().is_scored() {
        rows.push(row(
            if terms.allow_late {
                texts::BUTTON_POLICY_PENALTY
            } else {
                texts::BUTTON_POLICY_STRICT
            },
            TERMS_POLICY,
        ));
    }
    rows.push(row(
        if creating {
            texts::BUTTON_CANCEL_CREATION
        } else {
            texts::BUTTON_CANCEL_TERMS
        },
        TERMS_CANCEL,
    ));

    InlineKeyboardMarkup::new(rows)
}

pub(crate) fn terms_exit_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(texts::BUTTON_STAY, TERMS_STAY),
        row(texts::BUTTON_EXIT, TERMS_EXIT),
    ])
}

pub(super) fn terms_form_keyboard(
    form: &TermsForm,
    timezone: &BotTimezone,
) -> Option<InlineKeyboardMarkup> {
    let terms = &form.terms;
    let calendar = |month, after| calendar_keyboard(month, timezone.first_date_ending_after(after));
    let mut keyboard = match form.step {
        TermsStep::Policy => late_policy_keyboard(),
        TermsStep::Deadline => match (form.mode, &terms.target) {
            (InputMode::Calendar(month), TermsTarget::Edit { created_at, .. }) => {
                calendar(month, *created_at)
            }
            (InputMode::Calendar(month), TermsTarget::Create(_)) => calendar(month, Utc::now()),
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
                    let Ok(closes_at) = terms.preview_policy(periods).and_then(|policy| {
                        policy.validate_horizon(deadline)?;
                        policy.closes_at(deadline)
                    }) else {
                        continue;
                    };
                    rows.push(row(
                        texts::periods_option(
                            &texts::lateness_duration(terms.period_days, periods),
                            &timezone.format_until_short(closes_at),
                        ),
                        format!("{LATE_PERIODS_PREFIX}{periods}"),
                    ));
                }
            }
            rows.push(vec![
                button(
                    texts::BUTTON_OTHER_PERIODS,
                    format!("{LATE_PERIODS_PREFIX}manual"),
                ),
                button(texts::BUTTON_PICK_DATE, TERMS_DURATION_DATE),
            ]);
            InlineKeyboardMarkup::new(rows)
        }
        TermsStep::DurationConfirm { .. } => {
            InlineKeyboardMarkup::new(vec![row(texts::BUTTON_APPLY_PERIODS, TERMS_DURATION_APPLY)])
        }
        TermsStep::Summary => terms_keyboard(terms, &terms.complete()?, timezone),
    };
    if form.mode != InputMode::Manual && form.can_go_back() {
        keyboard
            .inline_keyboard
            .push(row(texts::BUTTON_BACK, TERMS_BACK));
    }
    Some(keyboard)
}

/// Для длинной шкалы нужен ручной ввод: кнопки на весь диапазон `0..=100` не помещаются.
pub(crate) fn review_score_keyboard(
    submission_id: SubmissionId,
    max_score: u16,
) -> InlineKeyboardMarkup {
    let score = |score: u16| {
        button(
            score.to_string(),
            format!("{REVIEW_SCORE_PREFIX}{}:{score}", submission_id.as_uuid()),
        )
    };

    let mut rows: Vec<Row> = (0..=max_score)
        .collect::<Vec<_>>()
        .chunks(6)
        .map(|chunk| chunk.iter().copied().map(score).collect())
        .collect();
    rows.push(row(
        texts::BUTTON_OTHER_AMOUNT,
        format!("{REVIEW_SCORE_PREFIX}{}:manual", submission_id.as_uuid()),
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
        button(texts::BUTTON_DELETE, FILE_CONFIRM_DELETE),
        button(texts::BUTTON_CANCEL, FILE_CANCEL_DELETE),
    ]])
}

pub(crate) fn group_invite_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        row(
            texts::BUTTON_ROTATE_INVITE,
            ScreenAction::RotateGroupInvite.callback(),
        ),
        row(
            texts::BUTTON_BACK_TO_COURSE,
            ReplayableAction::Course(course_id).callback(),
        ),
    ])
}

pub(crate) fn confirm_invite_rotation_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        button(texts::BUTTON_CONFIRM_ROTATE, INVITE_CONFIRM_ROTATE),
        button(texts::BUTTON_CANCEL, INVITE_CANCEL_ROTATE),
    ]])
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
            ReplayableAction::Course(course_id).callback(),
        ),
    )
}

pub(crate) fn assignment_students_keyboard(
    assignment_id: AssignmentId,
    students: &[AssignmentStudentSummary],
) -> InlineKeyboardMarkup {
    students_keyboard(
        students
            .iter()
            .map(|item| (item.student.id(), item.student.display_name().as_str())),
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
                format!("{STUDENT_PREFIX}{}", student_id.as_uuid()),
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
    use crate::bot::assignment_form::{AssignmentText, DraftAssignment};
    use crate::bot::texts::deadline_left;
    use crate::domain::{Grading, LatePolicy, MaxScore};

    fn scored_terms() -> AssignmentTerms {
        let mut draft = DraftAssignment::new(
            AssignmentText {
                course_id: TeachingCourseId::new(),
                title: "Task".into(),
                body: "Body".into(),
            },
            2,
        );
        draft.grading = Grading::scored(10).unwrap();
        AssignmentTerms {
            deadline_at: Some(crate::test_support::t0() + chrono::Duration::days(7)),
            allow_late: true,
            period_days: 1,
            penalty: Some(1),
            max_periods: Some(100),
            ..AssignmentTerms::creating(draft)
        }
    }

    #[test]
    fn duration_presets_start_after_the_deadline_and_respect_the_horizon() {
        use crate::bot::callbacks::{CallbackAction, DeadlineAction, FlowAction};
        let timezone = timezone();
        let mut terms = scored_terms();
        terms.deadline_at = timezone.parse_deadline("20.08.2027");
        terms.period_days = 7;
        let deadline = terms.deadline_at.unwrap();
        let policy = LatePolicy::new(7, 1, 1).unwrap();
        let keyboard = duration_options_keyboard(&terms, deadline, &timezone);
        let mut dates = Vec::new();
        for button in keyboard.inline_keyboard.iter().flatten() {
            let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
                panic!("expected a callback");
            };
            let action = crate::bot::callbacks::parse(data).expect("routable duration button");
            if let CallbackAction::Flow(FlowAction::Deadline(DeadlineAction::Pick(date))) = action {
                let at = timezone.end_of(date).unwrap();
                assert!(at > deadline);
                let periods = u16::try_from(policy.periods_to_close_at(deadline, at)).unwrap();
                LatePolicy::new(7, 1, periods)
                    .unwrap()
                    .validate_horizon(deadline)
                    .unwrap();
                dates.push((date, periods));
            }
        }
        assert_eq!(dates.len(), 3);
        assert_eq!(
            dates[0],
            (chrono::NaiveDate::from_ymd_opt(2027, 8, 27).unwrap(), 1)
        );
        assert_eq!(dates[1].1, 2);
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
            original_closes_at: terms.deadline_at.unwrap(),
        };
        let mut form = TermsForm::new(terms, TermsStep::Deadline);
        form.mode = InputMode::Calendar(timezone.local_date(created_at));
        let (_, keyboard) = terms_form_view(&form, &timezone).unwrap();
        let json = serde_json::to_string(&keyboard).unwrap();
        assert!(!json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-20")));
        assert!(json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-21")));
    }

    #[test]
    fn a_hundred_periods_still_produce_a_short_schedule() {
        let form = TermsForm::new(scored_terms(), TermsStep::Summary);
        let (text, keyboard) = terms_form_view(&form, &timezone()).unwrap();
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
        let policy = LatePolicy::new(7, 1, 3).unwrap();
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
        let policy = LatePolicy::new(1, 5, 40).unwrap();
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
    fn a_long_schedule_collapses_its_middle_into_one_line() {
        let timezone = timezone();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();
        let policy = LatePolicy::new(1, 1, 40).unwrap();
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
    fn calendar_return_and_previous_step_have_distinct_labels_and_actions() {
        let mut form = TermsForm::new(scored_terms(), TermsStep::Deadline);
        form.mode = InputMode::Calendar(timezone().today());
        let (_, keyboard) = terms_form_view(&form, &timezone()).unwrap();
        let json = serde_json::to_value(keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows[rows.len() - 2][0]["text"], texts::BUTTON_DATE_OPTIONS);
        assert_eq!(rows[rows.len() - 2][0]["callback_data"], DEADLINE_OPTIONS);
        assert_eq!(rows.last().unwrap()[0]["text"], texts::BUTTON_BACK);
        assert_eq!(rows.last().unwrap()[0]["callback_data"], TERMS_BACK);
    }

    fn timezone() -> BotTimezone {
        BotTimezone::new(Tz::Asia__Vladivostok)
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
            assert!(terms_form_view(&form, &timezone).is_none(), "{step:?}");
        }

        let mut terms = scored_terms();
        terms.deadline_at = None;
        let form = TermsForm::new(terms, TermsStep::DurationDate);
        assert!(terms_form_view(&form, &timezone).is_none());
    }

    #[test]
    fn button_labels_are_truncated_only_when_needed() {
        assert_eq!(button_label("Мои задания"), "Мои задания");
        assert_eq!(button_label(&"я".repeat(61)).chars().count(), 63);
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

        assert!(!callbacks.contains(&"cal:2026-07"));
        assert!(callbacks.contains(&"cal:2026-09"));
        assert!(!callbacks.contains(&"cal:pick:2026-08-12"));
        assert!(callbacks.contains(&"cal:pick:2026-08-13"));
        assert!(callbacks.contains(&"cal:pick:2026-08-31"));
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
                "dl:days:7",
                "dl:days:14",
                "dl:months:1",
                "dl:calendar",
                "dl:manual"
            ]
        );
    }

    #[test]
    fn passed_deadlines_are_not_reported_as_time_left() {
        let past: DateTime<Utc> = Utc::now() - chrono::Duration::hours(1);

        assert_eq!(deadline_left(past, Utc::now()), texts::DEADLINE_PASSED);
    }

    #[test]
    fn assignment_action_callbacks_fit_telegram_limit() {
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let draft = assignment(&course, &teacher).build();
        let published = assignment(&course, &teacher).published().build();
        let mut archived = published.clone();
        archived.archive(&course, &teacher).unwrap();

        for assignment in [draft, published, archived] {
            let json = serde_json::to_value(assignment_actions_keyboard(&assignment)).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    assert!(button["callback_data"].as_str().unwrap().len() <= 64);
                }
            }
        }
    }

    #[test]
    fn attachment_callbacks_fit_telegram_limit() {
        use crate::domain::{
            FilePresentation, StoredFileId, StoredFileMetadata, StoredFileOwner, StoredFileParts,
        };
        use crate::test_support::{assignment, course_for, student_group, teacher};

        let teacher = teacher(1);
        let group = student_group("test-group");
        let course = course_for(&group, &teacher);
        let assignment = assignment(&course, &teacher).build();
        let file = StoredFileMetadata::restore(StoredFileParts {
            id: StoredFileId::new(),
            owner: StoredFileOwner::Assignment(assignment.id()),
            position: 1,
            presentation: FilePresentation::Document,
            filename: "file.txt".to_owned(),
            mime_type: Some("text/plain".to_owned()),
            size_bytes: 4,
            sha256: [0; 32],
            created_at: Utc::now(),
        })
        .unwrap();

        for keyboard in [
            assignment_files_keyboard(&assignment, &[file], true, true),
            confirm_file_deletion_keyboard(),
        ] {
            let json = serde_json::to_value(keyboard).unwrap();
            for row in json["inline_keyboard"].as_array().unwrap() {
                for button in row.as_array().unwrap() {
                    assert!(button["callback_data"].as_str().unwrap().len() <= 64);
                }
            }
        }
    }
}
