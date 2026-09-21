use chrono::{Datelike, NaiveDate, Utc};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup};

use crate::app::{AssignmentStudentSummary, CourseStudent, TeachingCourseSummary};
use crate::domain::{
    Assignment, AssignmentStatus, Role, StoredFileMetadata, SubmissionId, TeachingCourseId,
};

use super::render::{assignment_status_icon, button_label};
use crate::bot::callbacks::{
    CALENDAR_MONTH_PREFIX, CALENDAR_NOOP, CALENDAR_PICK_PREFIX, DEADLINE_CALENDAR,
    DEADLINE_DAYS_PREFIX, DEADLINE_MANUAL, DEADLINE_MONTHS_PREFIX, DEADLINE_OPTIONS,
    FILE_CANCEL_DELETE, FILE_CONFIRM_DELETE, GRADING_PREFIX, INVITE_CANCEL_ROTATE,
    INVITE_CONFIRM_ROTATE, LATE_POLICY_PREFIX, PageTarget, REVIEW_ACCEPT_PREFIX,
    REVIEW_REJECT_PREFIX, REVIEW_SCORE_PREFIX, ReplayableAction, STUDENT_PREFIX, ScreenAction,
};
use crate::bot::callbacks::{
    LATE_PERIODS_PREFIX, PENALTY_PERIOD, PENALTY_PREFIX, TERMS_BACK, TERMS_CANCEL, TERMS_DEADLINE,
    TERMS_DURATION_APPLY, TERMS_DURATION_DATE, TERMS_PENALTY, TERMS_PERIOD, TERMS_PERIODS,
    TERMS_POLICY, TERMS_SAVE,
};
use crate::bot::state::{AssignmentTerms, InputMode, TermsForm, TermsStep, TermsTarget};
use crate::bot::texts;
use crate::bot::timezone::BotTimezone;

pub(in crate::bot) const BUTTON_ASSIGNMENTS: &str = "📚 Мои задания";
pub(in crate::bot) const BUTTON_ARCHIVE: &str = "🗄 Архив";
pub(in crate::bot) const BUTTON_QUEUE: &str = "⏳ Очередь проверки";
pub(in crate::bot) const BUTTON_REVIEW: &str = "✅ Проверить работы";
pub(in crate::bot) const BUTTON_COURSES: &str = "📘 Мои курсы";
pub(in crate::bot) const BUTTON_PROFILE: &str = "👤 Профиль";
pub(in crate::bot) const BUTTON_HELP: &str = "❓ Помощь";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::bot) enum MenuAction {
    Assignments,
    Archive,
    Queue,
    Review,
    Courses,
    Profile,
    Help,
}

pub(in crate::bot) fn menu_action(text: &str) -> Option<MenuAction> {
    match text.trim() {
        BUTTON_ASSIGNMENTS => Some(MenuAction::Assignments),
        BUTTON_ARCHIVE => Some(MenuAction::Archive),
        BUTTON_QUEUE => Some(MenuAction::Queue),
        BUTTON_REVIEW => Some(MenuAction::Review),
        BUTTON_COURSES => Some(MenuAction::Courses),
        BUTTON_PROFILE => Some(MenuAction::Profile),
        BUTTON_HELP => Some(MenuAction::Help),
        _ => None,
    }
}

pub(in crate::bot) fn main_keyboard(role: Role) -> KeyboardMarkup {
    match role {
        Role::Student { .. } => student_keyboard(),
        Role::Teacher | Role::Admin => teacher_keyboard(),
    }
}

pub(in crate::bot) fn main_menu(role: Role, pending: Option<u32>) -> InlineKeyboardMarkup {
    match role {
        Role::Student { .. } => student_menu(),
        Role::Teacher | Role::Admin => teacher_menu(pending),
    }
}

pub(in crate::bot) fn student_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![
            KeyboardButton::new(BUTTON_ASSIGNMENTS),
            KeyboardButton::new(BUTTON_QUEUE),
        ],
        vec![
            KeyboardButton::new(BUTTON_ARCHIVE),
            KeyboardButton::new(BUTTON_PROFILE),
        ],
        vec![KeyboardButton::new(BUTTON_HELP)],
    ])
}

pub(in crate::bot) fn teacher_keyboard() -> KeyboardMarkup {
    bottom_menu(vec![
        vec![KeyboardButton::new(BUTTON_REVIEW)],
        vec![
            KeyboardButton::new(BUTTON_COURSES),
            KeyboardButton::new(BUTTON_PROFILE),
        ],
        vec![KeyboardButton::new(BUTTON_HELP)],
    ])
}

fn bottom_menu(rows: Vec<Vec<KeyboardButton>>) -> KeyboardMarkup {
    KeyboardMarkup::new(rows)
        .resize_keyboard()
        .persistent()
        .input_field_placeholder("Выберите пункт меню")
}

pub(in crate::bot) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            BUTTON_ASSIGNMENTS,
            ReplayableAction::Page(PageTarget::Assignments, 0).callback(),
        )],
        vec![
            InlineKeyboardButton::callback(
                BUTTON_QUEUE,
                ReplayableAction::Page(PageTarget::Queue, 0).callback(),
            ),
            InlineKeyboardButton::callback(
                BUTTON_ARCHIVE,
                ReplayableAction::Page(PageTarget::Archive, 0).callback(),
            ),
        ],
        vec![InlineKeyboardButton::callback(
            BUTTON_PROFILE,
            ReplayableAction::Profile.callback(),
        )],
    ])
}

pub(in crate::bot) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || BUTTON_REVIEW.to_owned(),
        |pending| format!("✅ Проверить работы ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            review_label,
            ReplayableAction::NextReview.callback(),
        )],
        vec![InlineKeyboardButton::callback(
            BUTTON_COURSES,
            ReplayableAction::Page(PageTarget::Courses, 0).callback(),
        )],
        vec![InlineKeyboardButton::callback(
            BUTTON_PROFILE,
            ReplayableAction::Profile.callback(),
        )],
    ])
}

pub(in crate::bot) fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "✅ Принять",
            format!("{REVIEW_ACCEPT_PREFIX}{}", submission_id.as_uuid()),
        ),
        InlineKeyboardButton::callback(
            "❌ Отклонить",
            format!("{REVIEW_REJECT_PREFIX}{}", submission_id.as_uuid()),
        ),
    ]])
}

pub(in crate::bot) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        courses
            .iter()
            .map(|course| {
                vec![InlineKeyboardButton::callback(
                    button_label(&format!(
                        "📘 {} · {}",
                        course.subject.name().as_str(),
                        course.group.code().as_str()
                    )),
                    ReplayableAction::Course(course.course.id()).callback(),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

pub(in crate::bot) fn course_assignments_keyboard(
    course_id: TeachingCourseId,
    assignments: &[Assignment],
) -> InlineKeyboardMarkup {
    let mut rows: Vec<_> = assignments
        .iter()
        .map(|assignment| {
            vec![InlineKeyboardButton::callback(
                button_label(&format!(
                    "{} {}",
                    assignment_status_icon(assignment.status()),
                    assignment.title().as_str(),
                )),
                ReplayableAction::ManageAssignment(assignment.id()).callback(),
            )]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "➕ Создать задание",
        ReplayableAction::StartAssignmentCreation(course_id).callback(),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "📊 Оценки",
        ReplayableAction::Gradebook(course_id).callback(),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🔗 Ссылка для группы",
        ReplayableAction::GroupInvite(course_id).callback(),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К списку курсов",
        ReplayableAction::Page(PageTarget::Courses, 0).callback(),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn assignment_actions_keyboard(assignment: &Assignment) -> InlineKeyboardMarkup {
    let assignment_id = assignment.id();
    let mut rows = Vec::new();

    match assignment.status() {
        AssignmentStatus::Draft => {
            rows.push(vec![InlineKeyboardButton::callback(
                "🚀 Опубликовать",
                ScreenAction::Publish(assignment_id).callback(),
            )]);
            rows.push(vec![InlineKeyboardButton::callback(
                "📦 Архивировать",
                ScreenAction::Archive(assignment_id).callback(),
            )]);
        }
        AssignmentStatus::Published => rows.push(vec![InlineKeyboardButton::callback(
            "📦 Архивировать",
            ScreenAction::Archive(assignment_id).callback(),
        )]),
        AssignmentStatus::Archived => rows.push(vec![InlineKeyboardButton::callback(
            "🚀 Опубликовать снова",
            ScreenAction::Publish(assignment_id).callback(),
        )]),
    }

    if assignment.status() != AssignmentStatus::Archived {
        rows.push(vec![InlineKeyboardButton::callback(
            texts::BUTTON_EDIT_DATES,
            ScreenAction::ChangeDeadline(assignment_id).callback(),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "📎 Вложения",
        ReplayableAction::AssignmentFiles(assignment_id).callback(),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🎁 Выдать попытки",
        ReplayableAction::GrantAttempts(assignment_id).callback(),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К заданиям курса",
        ReplayableAction::Course(assignment.teaching_course_id()).callback(),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn deadline_options_keyboard(timezone: &BotTimezone) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    let mut preset = |label: &str, date: Option<NaiveDate>, callback: &str| {
        if let Some(date) = date {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("📅 {label} · {}", timezone.format_date(date)),
                callback.to_owned(),
            )]);
        }
    };

    preset(
        "Через неделю",
        timezone.in_days(7),
        &format!("{DEADLINE_DAYS_PREFIX}7"),
    );
    preset(
        "Через 2 недели",
        timezone.in_days(14),
        &format!("{DEADLINE_DAYS_PREFIX}14"),
    );
    preset(
        "Через месяц",
        timezone.in_months(1),
        &format!("{DEADLINE_MONTHS_PREFIX}1"),
    );
    rows.push(vec![InlineKeyboardButton::callback(
        "🗓 Выбрать дату",
        DEADLINE_CALENDAR,
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "✍️ Ввести вручную",
        DEADLINE_MANUAL,
    )]);

    InlineKeyboardMarkup::new(rows)
}

fn duration_options_keyboard(
    terms: &AssignmentTerms,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let deadline = terms.deadline_at.expect("duration follows deadline");
    let base = timezone.local_date(deadline - chrono::Duration::nanoseconds(1));
    let mut rows = Vec::new();
    for (label, date) in [
        (
            "Неделя после срока",
            base.checked_add_days(chrono::Days::new(7)),
        ),
        (
            "2 недели после срока",
            base.checked_add_days(chrono::Days::new(14)),
        ),
        (
            "Месяц после срока",
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
        rows.push(vec![InlineKeyboardButton::callback(
            format!("📅 {label} · {}", timezone.format_date(date)),
            format!("{CALENDAR_PICK_PREFIX}{date}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "🗓 Выбрать дату",
        DEADLINE_CALENDAR,
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "✍️ Ввести вручную",
        DEADLINE_MANUAL,
    )]);
    InlineKeyboardMarkup::new(rows)
}

const MONTHS: [&str; 12] = [
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

pub(in crate::bot) fn calendar_keyboard(
    month: NaiveDate,
    today: NaiveDate,
) -> InlineKeyboardMarkup {
    let blank = || InlineKeyboardButton::callback("·", CALENDAR_NOOP);
    let first = month.with_day(1).unwrap_or(month);
    let previous = first.checked_sub_months(chrono::Months::new(1));
    let next = first.checked_add_months(chrono::Months::new(1));

    let back = match previous {
        Some(previous) if previous >= today.with_day(1).unwrap_or(today) => {
            InlineKeyboardButton::callback(
                "◀️",
                format!("{CALENDAR_MONTH_PREFIX}{}", month_key(previous)),
            )
        }
        _ => blank(),
    };
    let forward = next.map_or_else(blank, |next| {
        InlineKeyboardButton::callback("▶️", format!("{CALENDAR_MONTH_PREFIX}{}", month_key(next)))
    });

    let mut rows = vec![
        vec![
            back,
            InlineKeyboardButton::callback(
                format!("{} {}", MONTHS[(first.month() - 1) as usize], first.year()),
                CALENDAR_NOOP,
            ),
            forward,
        ],
        ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"]
            .into_iter()
            .map(|day| InlineKeyboardButton::callback(day, CALENDAR_NOOP))
            .collect(),
    ];

    let mut week = vec![blank(); first.weekday().num_days_from_monday() as usize];
    let mut day = first;
    while day.month() == first.month() {
        week.push(if day < today {
            blank()
        } else if day == today {
            InlineKeyboardButton::callback(
                format!("»{}", day.day()),
                format!("{CALENDAR_PICK_PREFIX}{}", day.format("%Y-%m-%d")),
            )
        } else {
            InlineKeyboardButton::callback(
                day.day().to_string(),
                format!("{CALENDAR_PICK_PREFIX}{}", day.format("%Y-%m-%d")),
            )
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

    rows.push(vec![InlineKeyboardButton::callback(
        texts::BUTTON_DATE_OPTIONS,
        DEADLINE_OPTIONS,
    )]);

    InlineKeyboardMarkup::new(rows)
}

fn month_key(date: NaiveDate) -> String {
    date.format("%Y-%m").to_string()
}

pub(in crate::bot) fn attempts_keyboard(prefix: &str) -> InlineKeyboardMarkup {
    let button = |amount: u16| {
        InlineKeyboardButton::callback(amount.to_string(), format!("{prefix}{amount}"))
    };

    InlineKeyboardMarkup::new(vec![
        [1, 2, 3].into_iter().map(button).collect(),
        [5, 10].into_iter().map(button).collect(),
        vec![InlineKeyboardButton::callback(
            "✍️ Другое число",
            format!("{prefix}manual"),
        )],
    ])
}

pub(in crate::bot) fn grading_keyboard() -> InlineKeyboardMarkup {
    let preset = |max_score: u16| {
        InlineKeyboardButton::callback(
            format!("{max_score}-балльная"),
            format!("{GRADING_PREFIX}{max_score}"),
        )
    };

    InlineKeyboardMarkup::new(vec![
        vec![preset(5), preset(10)],
        vec![InlineKeyboardButton::callback(
            "✍️ Другой максимум",
            format!("{GRADING_PREFIX}manual"),
        )],
        vec![InlineKeyboardButton::callback(
            "⏭ Без баллов, зачёт/незачёт",
            format!("{GRADING_PREFIX}passfail"),
        )],
    ])
}

pub(in crate::bot) fn late_policy_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            texts::BUTTON_STRICT,
            format!("{LATE_POLICY_PREFIX}none"),
        )],
        vec![InlineKeyboardButton::callback(
            texts::BUTTON_LATE,
            format!("{LATE_POLICY_PREFIX}penalty"),
        )],
    ])
}

pub(in crate::bot) fn penalty_keyboard(period_days: u16) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        [1, 2]
            .into_iter()
            .map(|n| {
                InlineKeyboardButton::callback(
                    format!("На {}", texts::points(n)),
                    format!("{PENALTY_PREFIX}{n}"),
                )
            })
            .collect(),
        vec![InlineKeyboardButton::callback(
            texts::BUTTON_OTHER_NUMBER,
            format!("{PENALTY_PREFIX}manual"),
        )],
        vec![InlineKeyboardButton::callback(
            format!("Изменить период: {period_days} дн."),
            PENALTY_PERIOD,
        )],
    ])
}

pub(in crate::bot) fn terms_keyboard(
    terms: &AssignmentTerms,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let mut rows = vec![vec![InlineKeyboardButton::callback(
        if matches!(terms.target, TermsTarget::Create(_)) {
            texts::BUTTON_SAVE_DRAFT
        } else {
            texts::BUTTON_SAVE_TERMS
        },
        TERMS_SAVE,
    )]];
    rows.push(vec![InlineKeyboardButton::callback(
        format!(
            "{} {}",
            texts::SUBMIT_UNTIL,
            terms
                .deadline_at
                .map(|date| format!("до {}", timezone.format_until_short(date)))
                .unwrap_or_default()
        ),
        TERMS_DEADLINE,
    )]);
    if let Some(policy) = terms
        .committed_policy()
        .expect("summary requires complete policy")
    {
        rows.push(vec![
            InlineKeyboardButton::callback(
                format!("{} −{}", texts::PENALTY_LABEL, policy.penalty()),
                TERMS_PENALTY,
            ),
            InlineKeyboardButton::callback(
                format!("{} {} дн.", texts::PERIOD_LABEL, terms.period_days),
                TERMS_PERIOD,
            ),
        ]);
        rows.push(vec![InlineKeyboardButton::callback(
            format!(
                "{} {}",
                texts::LATENESS_LABEL,
                texts::lateness_duration(policy.period_days(), policy.max_periods())
            ),
            TERMS_PERIODS,
        )]);
    }
    if terms.grading().is_scored() {
        rows.push(vec![InlineKeyboardButton::callback(
            if terms.allow_late {
                "После срока: со штрафом"
            } else {
                "После срока: не принимать"
            },
            TERMS_POLICY,
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        if matches!(terms.target, TermsTarget::Create(_)) {
            texts::BUTTON_CANCEL_CREATION
        } else {
            texts::BUTTON_CANCEL_TERMS
        },
        TERMS_CANCEL,
    )]);
    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn terms_exit_keyboard() -> InlineKeyboardMarkup {
    use crate::bot::callbacks::{TERMS_EXIT, TERMS_STAY};
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            texts::BUTTON_STAY,
            TERMS_STAY,
        )],
        vec![InlineKeyboardButton::callback(
            texts::BUTTON_EXIT,
            TERMS_EXIT,
        )],
    ])
}

pub(super) fn terms_form_keyboard(
    form: &TermsForm,
    timezone: &BotTimezone,
) -> InlineKeyboardMarkup {
    let terms = &form.terms;
    let button = |label: &str, data: &str| InlineKeyboardButton::callback(label, data);
    let empty = || InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new());
    let date_keyboard = || {
        if let InputMode::Calendar(month) = form.mode {
            let after = if form.step == TermsStep::DurationDate {
                terms.deadline_at.expect("duration follows deadline")
            } else {
                match terms.target {
                    TermsTarget::Edit { created_at, .. } => created_at,
                    _ => Utc::now(),
                }
            };
            let earliest = timezone.first_date_ending_after(after);
            calendar_keyboard(month, earliest)
        } else if form.step == TermsStep::DurationDate {
            duration_options_keyboard(terms, timezone)
        } else {
            deadline_options_keyboard(timezone)
        }
    };
    let mut keyboard = match form.step {
        TermsStep::Policy => late_policy_keyboard(),
        TermsStep::Deadline | TermsStep::DurationDate => date_keyboard(),
        TermsStep::Penalty => penalty_keyboard(terms.period_days),
        TermsStep::Period => empty(),
        TermsStep::Periods => {
            let mut rows = Vec::new();
            if let Some(deadline) = terms.deadline_at {
                for periods in [1, 2, 4] {
                    let Ok(policy) = terms.preview_policy(periods) else {
                        continue;
                    };
                    if policy.validate_horizon(deadline).is_ok() {
                        let date = policy.closes_at(deadline).expect("validated horizon");
                        rows.push(vec![InlineKeyboardButton::callback(
                            format!(
                                "{} · до {}",
                                texts::lateness_duration(terms.period_days, periods),
                                timezone.format_until_short(date)
                            ),
                            format!("{LATE_PERIODS_PREFIX}{periods}"),
                        )]);
                    }
                }
            }
            rows.push(vec![
                button(
                    "Другое число периодов",
                    &format!("{LATE_PERIODS_PREFIX}manual"),
                ),
                button("Выбрать дату", TERMS_DURATION_DATE),
            ]);
            InlineKeyboardMarkup::new(rows)
        }
        TermsStep::DurationConfirm { .. } => InlineKeyboardMarkup::new(vec![vec![button(
            "Применить число периодов",
            TERMS_DURATION_APPLY,
        )]]),
        TermsStep::Summary => terms_keyboard(terms, timezone),
    };
    if form.mode != InputMode::Manual && form.can_go_back() {
        keyboard
            .inline_keyboard
            .push(vec![button(texts::BUTTON_BACK, TERMS_BACK)]);
    }
    keyboard
}

/// Кнопками балл вводится только на короткой шкале: `0..=100` в клавиатуру не помещается.
pub(in crate::bot) fn review_score_keyboard(
    submission_id: SubmissionId,
    max_score: u16,
) -> InlineKeyboardMarkup {
    let button = |score: u16| {
        InlineKeyboardButton::callback(
            score.to_string(),
            format!("{REVIEW_SCORE_PREFIX}{}:{score}", submission_id.as_uuid()),
        )
    };

    let mut rows: Vec<Vec<_>> = (0..=max_score)
        .collect::<Vec<_>>()
        .chunks(6)
        .map(|chunk| chunk.iter().copied().map(button).collect())
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "✍️ Другое число",
        format!("{REVIEW_SCORE_PREFIX}{}:manual", submission_id.as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn assignment_files_keyboard(
    assignment: &Assignment,
    files: &[StoredFileMetadata],
    can_add: bool,
    creation_flow: bool,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for (index, file) in files.iter().enumerate() {
        rows.push(vec![
            InlineKeyboardButton::callback(
                button_label(&format!("📄 {}", file.filename().as_str())),
                ScreenAction::OpenFile(index).callback(),
            ),
            InlineKeyboardButton::callback("🗑", ScreenAction::DeleteFile(index).callback()),
        ]);
    }
    if can_add {
        rows.push(vec![InlineKeyboardButton::callback(
            "📎 Добавить вложение",
            ScreenAction::AddFile.callback(),
        )]);
    }
    if creation_flow {
        rows.push(vec![InlineKeyboardButton::callback(
            "✅ Завершить создание",
            ScreenAction::FinishFileCreation.callback(),
        )]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(
            "⬅️ К заданию",
            ReplayableAction::ManageAssignment(assignment.id()).callback(),
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn confirm_file_deletion_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🗑 Удалить", FILE_CONFIRM_DELETE),
        InlineKeyboardButton::callback("↩️ Отмена", FILE_CANCEL_DELETE),
    ]])
}

pub(in crate::bot) fn group_invite_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "♻️ Перевыпустить ссылку",
            ScreenAction::RotateGroupInvite.callback(),
        )],
        vec![InlineKeyboardButton::callback(
            "⬅️ К курсу",
            ReplayableAction::Course(course_id).callback(),
        )],
    ])
}

pub(in crate::bot) fn confirm_invite_rotation_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("♻️ Перевыпустить", INVITE_CONFIRM_ROTATE),
        InlineKeyboardButton::callback("↩️ Отмена", INVITE_CANCEL_ROTATE),
    ]])
}

pub(in crate::bot) fn gradebook_students_keyboard(
    course_id: TeachingCourseId,
    students: &[CourseStudent],
) -> InlineKeyboardMarkup {
    let mut rows = students
        .iter()
        .map(|student| {
            vec![InlineKeyboardButton::callback(
                button_label(&format!("👤 {}", student.display_name,)),
                format!("{STUDENT_PREFIX}{}", student.student_id.as_uuid()),
            )]
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К курсу",
        ReplayableAction::Course(course_id).callback(),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(in crate::bot) fn paginated_keyboard(
    mut keyboard: InlineKeyboardMarkup,
    target: crate::bot::callbacks::PageTarget,
    page: u32,
    next: Option<u32>,
) -> InlineKeyboardMarkup {
    let mut buttons = Vec::new();
    if let Some(previous) = page.checked_sub(1) {
        buttons.push(InlineKeyboardButton::callback(
            "◀️ Назад",
            target.callback(previous),
        ));
    }
    if let Some(next) = next {
        buttons.push(InlineKeyboardButton::callback(
            "Вперёд ▶️",
            target.callback(next),
        ));
    }
    if !buttons.is_empty() {
        keyboard.inline_keyboard.push(buttons);
    }
    keyboard
}

pub(in crate::bot) fn assignment_students_keyboard(
    assignment_id: crate::domain::AssignmentId,
    students: &[AssignmentStudentSummary],
) -> InlineKeyboardMarkup {
    let mut rows = students
        .iter()
        .map(|student| {
            vec![InlineKeyboardButton::callback(
                button_label(&format!("👤 {}", student.student.display_name().as_str())),
                format!("{STUDENT_PREFIX}{}", student.student.id().as_uuid()),
            )]
        })
        .collect::<Vec<_>>();
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К заданию",
        ReplayableAction::ManageAssignment(assignment_id).callback(),
    )]);

    InlineKeyboardMarkup::new(rows)
}

#[cfg(test)]
mod tests {
    use super::super::render::{deadline_left, penalty_schedule, terms_form_view};
    use crate::domain::{Grading, LatePolicy, MaxScore};
    use chrono::{DateTime, FixedOffset};

    use super::*;

    fn scored_terms() -> AssignmentTerms {
        let mut draft = crate::bot::state::DraftAssignment::new(
            TeachingCourseId::new(),
            "Task".into(),
            "Body".into(),
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
        let keyboard = duration_options_keyboard(&terms, &timezone);
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
            assignment_id: crate::domain::AssignmentId::new(),
            grading: terms.grading(),
            revision: 0,
            created_at,
            original_closes_at: terms.deadline_at.unwrap(),
        };
        let mut form = TermsForm::new(terms, TermsStep::Deadline);
        form.mode = InputMode::Calendar(timezone.local_date(created_at));
        let (_, keyboard) = terms_form_view(&form, &timezone);
        let json = serde_json::to_string(&keyboard).unwrap();
        assert!(!json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-20")));
        assert!(json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-21")));
    }

    #[test]
    fn a_hundred_periods_still_produce_a_short_schedule() {
        let form = TermsForm::new(scored_terms(), TermsStep::Summary);
        let (text, keyboard) = terms_form_view(&form, &timezone());
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
        let max_score = crate::domain::MaxScore::new(10).unwrap();

        let schedule = penalty_schedule(deadline, policy, max_score, &timezone);

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

        let schedule = penalty_schedule(deadline, policy, max_score, &timezone);
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

        let schedule = penalty_schedule(deadline, policy, max_score, &timezone);
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
        let (_, keyboard) = terms_form_view(&form, &timezone());
        let json = serde_json::to_value(keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows[rows.len() - 2][0]["text"], texts::BUTTON_DATE_OPTIONS);
        assert_eq!(rows[rows.len() - 2][0]["callback_data"], DEADLINE_OPTIONS);
        assert_eq!(rows.last().unwrap()[0]["text"], texts::BUTTON_BACK);
        assert_eq!(rows.last().unwrap()[0]["callback_data"], TERMS_BACK);
    }

    fn timezone() -> BotTimezone {
        BotTimezone::new(FixedOffset::east_opt(10 * 3600).unwrap())
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
            menu_action(BUTTON_ASSIGNMENTS),
            Some(MenuAction::Assignments)
        );
        assert_eq!(menu_action(BUTTON_QUEUE), Some(MenuAction::Queue));
        assert_eq!(menu_action(BUTTON_REVIEW), Some(MenuAction::Review));
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
