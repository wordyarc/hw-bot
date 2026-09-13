use super::Bot;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::ChatId;
use teloxide::types::{
    InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, KeyboardRemove,
};

use crate::app::{AssignmentStudentSummary, CourseStudent, TeachingCourseSummary};
use crate::domain::{
    Assignment, AssignmentStatus, Grade, Grading, LatePolicy, Role, StoredFileMetadata,
    SubmissionId, SubmissionStatus, TeachingCourseId,
};

use super::HandlerResult;
use super::callbacks::{
    ARCHIVE_PREFIX, ATTEMPTS_PREFIX, CALENDAR_MONTH_PREFIX, CALENDAR_NOOP, CALENDAR_PICK_PREFIX,
    COURSE_PREFIX, CREATE_PREFIX, DEADLINE_CALENDAR, DEADLINE_DAYS_PREFIX, DEADLINE_MANUAL,
    DEADLINE_MONTHS_PREFIX, DEADLINE_OPTIONS, DEADLINE_PREFIX, FILE_ADD, FILE_CANCEL_DELETE,
    FILE_CONFIRM_DELETE, FILE_DELETE_PREFIX, FILE_FINISH, FILE_OPEN_PREFIX, FILES_PREFIX,
    GRADES_PREFIX, GRADING_PREFIX, INVITE_CANCEL_ROTATE, INVITE_CONFIRM_ROTATE, INVITE_PREFIX,
    INVITE_ROTATE, LATE_POLICY_PREFIX, MANAGE_PREFIX, MENU_ARCHIVE, MENU_ASSIGNMENTS, MENU_COURSES,
    MENU_PROFILE, MENU_QUEUE, NEXT_REVIEW, PUBLISH_PREFIX, REVIEW_ACCEPT_PREFIX,
    REVIEW_REJECT_PREFIX, REVIEW_SCORE_PREFIX, STUDENT_PREFIX,
};
use super::callbacks::{
    LATE_PERIODS_PREFIX, PENALTY_PERIOD, PENALTY_PREFIX, TERMS_BACK, TERMS_CANCEL, TERMS_DEADLINE,
    TERMS_DURATION_APPLY, TERMS_DURATION_DATE, TERMS_PENALTY, TERMS_PERIOD, TERMS_PERIODS,
    TERMS_POLICY, TERMS_SAVE,
};
use super::state::{AssignmentTerms, TermsForm, TermsStep, TermsTarget};
use super::texts;
use super::timezone::BotTimezone;

const MESSAGE_LIMIT: usize = 4096;

pub(super) const BUTTON_ASSIGNMENTS: &str = "📚 Мои задания";
pub(super) const BUTTON_ARCHIVE: &str = "🗄 Архив";
pub(super) const BUTTON_QUEUE: &str = "⏳ Очередь проверки";
pub(super) const BUTTON_REVIEW: &str = "✅ Проверить работы";
pub(super) const BUTTON_COURSES: &str = "📘 Мои курсы";
pub(super) const BUTTON_PROFILE: &str = "👤 Профиль";
pub(super) const BUTTON_HELP: &str = "❓ Помощь";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MenuAction {
    Assignments,
    Archive,
    Queue,
    Review,
    Courses,
    Profile,
    Help,
}

pub(super) fn menu_action(text: &str) -> Option<MenuAction> {
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

pub(super) fn main_keyboard(role: Role) -> KeyboardMarkup {
    match role {
        Role::Student { .. } => student_keyboard(),
        Role::Teacher | Role::Admin => teacher_keyboard(),
    }
}

pub(super) fn student_keyboard() -> KeyboardMarkup {
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

pub(super) fn teacher_keyboard() -> KeyboardMarkup {
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

pub(super) fn student_menu() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            BUTTON_ASSIGNMENTS,
            MENU_ASSIGNMENTS,
        )],
        vec![
            InlineKeyboardButton::callback(BUTTON_QUEUE, MENU_QUEUE),
            InlineKeyboardButton::callback(BUTTON_ARCHIVE, MENU_ARCHIVE),
        ],
        vec![InlineKeyboardButton::callback(BUTTON_PROFILE, MENU_PROFILE)],
    ])
}

pub(super) fn teacher_menu(pending: Option<u32>) -> InlineKeyboardMarkup {
    let review_label = pending.map_or_else(
        || BUTTON_REVIEW.to_owned(),
        |pending| format!("✅ Проверить работы ({pending})"),
    );
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(review_label, NEXT_REVIEW)],
        vec![InlineKeyboardButton::callback(BUTTON_COURSES, MENU_COURSES)],
        vec![InlineKeyboardButton::callback(BUTTON_PROFILE, MENU_PROFILE)],
    ])
}

pub(super) fn review_keyboard(submission_id: SubmissionId) -> InlineKeyboardMarkup {
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

pub(super) fn courses_keyboard(courses: &[TeachingCourseSummary]) -> InlineKeyboardMarkup {
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
                    format!("{COURSE_PREFIX}{}", course.course.id().as_uuid()),
                )]
            })
            .collect::<Vec<_>>(),
    )
}

pub(super) fn course_assignments_keyboard(
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
                format!("{MANAGE_PREFIX}{}", assignment.id().as_uuid()),
            )]
        })
        .collect();
    rows.push(vec![InlineKeyboardButton::callback(
        "➕ Создать задание",
        format!("{CREATE_PREFIX}{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "📊 Оценки",
        format!("{GRADES_PREFIX}{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🔗 Ссылка для группы",
        format!("{INVITE_PREFIX}{}", course_id.as_uuid()),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К списку курсов",
        MENU_COURSES,
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn assignment_actions_keyboard(assignment: &Assignment) -> InlineKeyboardMarkup {
    let assignment_id = assignment.id().as_uuid();
    let mut rows = Vec::new();

    match assignment.status() {
        AssignmentStatus::Draft => {
            rows.push(vec![InlineKeyboardButton::callback(
                "🚀 Опубликовать",
                format!("{PUBLISH_PREFIX}{assignment_id}"),
            )]);
            rows.push(vec![InlineKeyboardButton::callback(
                "📦 Архивировать",
                format!("{ARCHIVE_PREFIX}{assignment_id}"),
            )]);
        }
        AssignmentStatus::Published => rows.push(vec![InlineKeyboardButton::callback(
            "📦 Архивировать",
            format!("{ARCHIVE_PREFIX}{assignment_id}"),
        )]),
        AssignmentStatus::Archived => rows.push(vec![InlineKeyboardButton::callback(
            "🚀 Опубликовать снова",
            format!("{PUBLISH_PREFIX}{assignment_id}"),
        )]),
    }

    if assignment.status() != AssignmentStatus::Archived {
        rows.push(vec![InlineKeyboardButton::callback(
            texts::BUTTON_EDIT_DATES,
            format!("{DEADLINE_PREFIX}{assignment_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "📎 Вложения",
        format!("{FILES_PREFIX}{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "🎁 Выдать попытки",
        format!("{ATTEMPTS_PREFIX}{assignment_id}"),
    )]);
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ К заданиям курса",
        format!(
            "{COURSE_PREFIX}{}",
            assignment.teaching_course_id().as_uuid()
        ),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn deadline_options_keyboard(timezone: &BotTimezone) -> InlineKeyboardMarkup {
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
    let base = timezone.local_date(deadline);
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
        let policy = LatePolicy::new(terms.period_days, terms.penalty.unwrap_or(1), 1)
            .expect("valid period and penalty");
        let Ok(periods) = u16::try_from(policy.periods_until(deadline, at)) else {
            continue;
        };
        if LatePolicy::new(policy.period_days(), policy.penalty(), periods)
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

pub(super) fn calendar_keyboard(month: NaiveDate, today: NaiveDate) -> InlineKeyboardMarkup {
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

pub(super) fn attempts_keyboard(prefix: &str) -> InlineKeyboardMarkup {
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

pub(super) fn grading_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn late_policy_keyboard() -> InlineKeyboardMarkup {
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

pub(super) fn penalty_keyboard(period_days: u16) -> InlineKeyboardMarkup {
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

pub(super) fn terms_keyboard(
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
                .map(|date| timezone.format_short(date))
                .unwrap_or_default()
        ),
        TERMS_DEADLINE,
    )]);
    if terms.allow_late {
        rows.push(vec![
            InlineKeyboardButton::callback(
                format!("{} −{}", texts::PENALTY_LABEL, terms.penalty.unwrap_or(0)),
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
                texts::lateness_duration(terms.period_days, terms.max_periods.unwrap_or(0))
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

pub(super) fn terms_exit_keyboard() -> InlineKeyboardMarkup {
    use super::callbacks::{TERMS_EXIT, TERMS_STAY};
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

pub(super) fn terms_form_view(
    form: &TermsForm,
    timezone: &BotTimezone,
) -> (String, InlineKeyboardMarkup) {
    let terms = &form.terms;
    let button = |label: &str, data: &str| InlineKeyboardButton::callback(label, data);
    let empty = || InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new());
    let context = terms
        .deadline_at
        .map(|deadline| {
            format!(
                "{} {}.\n",
                if terms.allow_late {
                    texts::NO_PENALTY_UNTIL
                } else {
                    texts::SUBMIT_UNTIL
                },
                timezone.format(deadline)
            )
        })
        .unwrap_or_default();
    let date_keyboard = || {
        if let Some(month) = form.calendar_month {
            let after = if form.step == TermsStep::DurationDate {
                terms.deadline_at.expect("duration follows deadline")
            } else {
                match terms.target {
                    TermsTarget::Edit { created_at, .. } => created_at,
                    _ => Utc::now(),
                }
            };
            let earliest = timezone.first_date_after(after).unwrap_or(NaiveDate::MAX);
            calendar_keyboard(month, earliest)
        } else if form.step == TermsStep::DurationDate {
            duration_options_keyboard(terms, timezone)
        } else {
            deadline_options_keyboard(timezone)
        }
    };
    let (text, mut keyboard) = match form.step {
        TermsStep::Policy => (
            texts::LATE_POLICY_QUESTION.to_owned(),
            late_policy_keyboard(),
        ),
        TermsStep::Deadline | TermsStep::DurationDate => {
            let question = if form.step == TermsStep::DurationDate {
                texts::DURATION_DATE_QUESTION
            } else if terms.allow_late {
                texts::NO_PENALTY_QUESTION
            } else {
                texts::DEADLINE_QUESTION
            };
            (
                format!(
                    "{context}{question}\nВремя по умолчанию — {} ({}). Можно прислать дату сообщением.\n{}",
                    timezone.deadline_time().format("%H:%M"),
                    timezone.suffix(),
                    texts::DATE_FORMAT
                ),
                date_keyboard(),
            )
        }
        TermsStep::Penalty => (
            format!(
                "{context}{}\nРазмер штрафа в баллах за каждые начавшиеся {} дн. Пришлите число от 1 до 100 сообщением.",
                texts::penalty_question(terms.period_days),
                terms.period_days
            ),
            penalty_keyboard(terms.period_days),
        ),
        TermsStep::Period => (format!("{context}{}", texts::PERIOD_PROMPT), empty()),
        TermsStep::Periods => {
            let mut rows = Vec::new();
            if let Some(deadline) = terms.deadline_at {
                for periods in [1, 2, 4] {
                    let policy =
                        LatePolicy::new(terms.period_days, terms.penalty.unwrap_or(1), periods)
                            .expect("valid preset");
                    if policy.validate_horizon(deadline).is_ok() {
                        let date = policy.closes_at(deadline).expect("validated horizon");
                        rows.push(vec![InlineKeyboardButton::callback(
                            format!(
                                "{} · до {}",
                                texts::lateness_duration(terms.period_days, periods),
                                timezone.format_short(date)
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
            (
                format!(
                    "{context}{}\n{}",
                    texts::penalty_rule(terms.period_days, terms.penalty.unwrap_or(0)),
                    texts::periods_question(terms.period_days)
                ),
                InlineKeyboardMarkup::new(rows),
            )
        }
        TermsStep::DurationConfirm { selected, periods } => {
            let policy = LatePolicy::new(terms.period_days, terms.penalty.unwrap_or(1), periods)
                .expect("validated duration");
            let closing = policy
                .closes_at(terms.deadline_at.expect("duration follows deadline"))
                .expect("validated duration");
            (
                format!(
                    "{} — это {}.\n{} {} включительно. Применить?",
                    timezone.format(selected),
                    texts::lateness_duration(terms.period_days, periods),
                    texts::SUBMISSIONS_CLOSE_LABEL,
                    timezone.format(closing)
                ),
                InlineKeyboardMarkup::new(vec![vec![button(
                    "Применить число периодов",
                    TERMS_DURATION_APPLY,
                )]]),
            )
        }
        TermsStep::Summary => {
            let deadline = terms.deadline_at.expect("summary requires deadline");
            let policy = terms
                .late_policy()
                .expect("summary requires complete policy");
            let closing = policy.map_or(deadline, |policy| {
                policy.closes_at(deadline).expect("validated horizon")
            });
            let mut text = format!(
                "{}\n{}",
                texts::TERMS_CONFIRM,
                terms_lines(terms.grading(), deadline, policy, timezone)
            );
            if let (Some(policy), Grading::Score { max_score }) = (policy, terms.grading()) {
                text.push_str(&format!(
                    "\n{}",
                    texts::score_preview(
                        policy.possible_score(max_score, 1),
                        policy.possible_score(max_score, u32::from(policy.max_periods()))
                    )
                ));
            }
            let previous = match terms.target {
                TermsTarget::Edit {
                    original_closes_at, ..
                } => Some(original_closes_at),
                _ => form.previous_closes_at,
            };
            if let Some(previous) = previous
                && closing != previous
            {
                text.push_str(&format!(
                    "\n{}",
                    texts::closing_change(&timezone.format(previous), &timezone.format(closing))
                ));
            }
            if matches!(terms.target, TermsTarget::Edit { .. }) && deadline <= Utc::now() {
                text.push_str(&format!(
                    "\n{}",
                    if policy.is_some() {
                        texts::PAST_PENALTY_WARNING
                    } else {
                        texts::PAST_STRICT_WARNING
                    }
                ));
            }
            (text, terms_keyboard(terms, timezone))
        }
    };
    if !form.manual && form.can_go_back() {
        keyboard
            .inline_keyboard
            .push(vec![button(texts::BUTTON_BACK, TERMS_BACK)]);
    }
    (text, keyboard)
}

/// Кнопками балл вводится только на короткой шкале: `0..=100` в клавиатуру не помещается.
pub(super) fn review_score_keyboard(
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

pub(super) fn assignment_files_keyboard(
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
                format!("{FILE_OPEN_PREFIX}{index}"),
            ),
            InlineKeyboardButton::callback("🗑", format!("{FILE_DELETE_PREFIX}{index}")),
        ]);
    }
    if can_add {
        rows.push(vec![InlineKeyboardButton::callback(
            "📎 Добавить вложение",
            FILE_ADD,
        )]);
    }
    if creation_flow {
        rows.push(vec![InlineKeyboardButton::callback(
            "✅ Завершить создание",
            FILE_FINISH,
        )]);
    } else {
        rows.push(vec![InlineKeyboardButton::callback(
            "⬅️ К заданию",
            format!("{MANAGE_PREFIX}{}", assignment.id().as_uuid()),
        )]);
    }

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn confirm_file_deletion_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🗑 Удалить", FILE_CONFIRM_DELETE),
        InlineKeyboardButton::callback("↩️ Отмена", FILE_CANCEL_DELETE),
    ]])
}

pub(super) fn group_invite_keyboard(course_id: TeachingCourseId) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "♻️ Перевыпустить ссылку",
            INVITE_ROTATE,
        )],
        vec![InlineKeyboardButton::callback(
            "⬅️ К курсу",
            format!("{COURSE_PREFIX}{}", course_id.as_uuid()),
        )],
    ])
}

pub(super) fn confirm_invite_rotation_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("♻️ Перевыпустить", INVITE_CONFIRM_ROTATE),
        InlineKeyboardButton::callback("↩️ Отмена", INVITE_CANCEL_ROTATE),
    ]])
}

pub(super) fn format_file_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} Б")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} КиБ", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МиБ", size_bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(super) fn gradebook_students_keyboard(
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
        format!("{COURSE_PREFIX}{}", course_id.as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn paginated_keyboard(
    mut keyboard: InlineKeyboardMarkup,
    target: super::callbacks::PageTarget,
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

pub(super) fn assignment_students_keyboard(
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
        format!("{MANAGE_PREFIX}{}", assignment_id.as_uuid()),
    )]);

    InlineKeyboardMarkup::new(rows)
}

pub(super) fn deadline_left(deadline: chrono::DateTime<Utc>, now: chrono::DateTime<Utc>) -> String {
    let left = deadline - now;

    if left.num_seconds() <= 0 {
        return "дедлайн прошёл".to_owned();
    }

    let days = left.num_days();
    let hours = left.num_hours() % 24;
    if days > 0 {
        format!("осталось {days} д {hours} ч")
    } else if left.num_hours() > 0 {
        format!("осталось {} ч", left.num_hours())
    } else {
        format!("осталось {} мин", left.num_minutes().max(1))
    }
}

pub(super) fn grading_lines(
    assignment: &Assignment,
    now: DateTime<Utc>,
    timezone: &BotTimezone,
) -> String {
    let mut lines = format!(
        "\n{}",
        terms_lines(
            assignment.grading(),
            assignment.deadline_at(),
            assignment.late_policy(),
            timezone
        )
    );
    if now > assignment.closes_at() {
        lines.push_str(&format!(
            "\n{} {}.",
            texts::CLOSED_ON,
            timezone.format(assignment.closes_at())
        ));
    }

    let penalty = assignment.penalty_at(now);
    if penalty > 0 && now <= assignment.closes_at() {
        lines.push_str(&format!(
            "\n{} −{penalty}",
            super::texts::LATE_PENALTY_LABEL
        ));
    }

    lines
}

pub(super) fn terms_lines(
    grading: Grading,
    deadline_at: DateTime<Utc>,
    late_policy: Option<LatePolicy>,
    timezone: &BotTimezone,
) -> String {
    let scale = match grading.max_score() {
        Some(max) => format!("{} {}", texts::MAX_SCORE_LABEL, texts::points(max)),
        None => texts::PASS_FAIL_LABEL.to_owned(),
    };
    match late_policy {
        Some(policy) => format!(
            "{scale}\n{} {}.\n{}\n{} {}.\n{} {} включительно.",
            texts::NO_PENALTY_UNTIL,
            timezone.format(deadline_at),
            texts::penalty_rule(policy.period_days(), policy.penalty()),
            texts::LATENESS_LABEL,
            texts::lateness_duration(policy.period_days(), policy.max_periods()),
            texts::SUBMISSIONS_CLOSE_LABEL,
            timezone.format(policy.closes_at(deadline_at).expect("validated horizon"))
        ),
        None => format!(
            "{scale}\n{} {}. {}",
            texts::SUBMIT_UNTIL,
            timezone.format(deadline_at),
            texts::STRICT_AFTER_DEADLINE
        ),
    }
}

pub(super) fn grade_line(grade: Grade) -> String {
    if grade.penalty() == 0 {
        return format!("🏅 Балл: {} из {}", grade.final_score(), grade.max_score());
    }

    format!(
        "🏅 Балл: {} − {} = {} из {} (штраф за опоздание)",
        grade.raw_score(),
        grade.penalty(),
        grade.final_score(),
        grade.max_score(),
    )
}

pub(super) fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "⏳ на проверке",
        SubmissionStatus::Accepted => "✅ принята",
        SubmissionStatus::Rejected => "❌ отклонена",
    }
}

pub(super) fn assignment_status(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️ черновик",
        AssignmentStatus::Published => "🟢 опубликовано",
        AssignmentStatus::Archived => "📦 архив",
    }
}

fn assignment_status_icon(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️",
        AssignmentStatus::Published => "🟢",
        AssignmentStatus::Archived => "📦",
    }
}

pub(super) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Student { .. } => "🎓 студент",
        Role::Teacher => "👨‍🏫 преподаватель",
        Role::Admin => "🛠 администратор",
    }
}

pub(super) fn button_label(label: &str) -> String {
    let truncated = label.chars().count() > 60;
    let mut label: String = label.chars().take(60).collect();
    if truncated {
        label.push_str("...");
    }
    label
}

fn message_chunks(text: String) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;

    for character in text.chars() {
        if current_len == MESSAGE_LIMIT {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        current.push(character);
        current_len += 1;
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

pub(super) async fn send_message_chunks(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    keyboard: Option<InlineKeyboardMarkup>,
) -> HandlerResult {
    let chunks = message_chunks(text);
    let last = chunks.len().saturating_sub(1);

    for (index, chunk) in chunks.into_iter().enumerate() {
        if index == last
            && let Some(keyboard) = keyboard.clone()
        {
            bot.send_message(chat_id, chunk)
                .reply_markup(keyboard)
                .await?;
        } else {
            bot.send_message(chat_id, chunk).await?;
        }
    }

    Ok(())
}

pub(super) async fn ask_input(bot: &Bot, chat_id: ChatId, prompt: &str) -> HandlerResult {
    bot.send_message(chat_id, super::texts::input_prompt(prompt))
        .reply_markup(KeyboardRemove::new())
        .await?;
    Ok(())
}

/// Остаётся только там, где кнопка не может сработать: у мутаций и у кнопок, чей смысл
/// лежит в стейте диалога
pub(super) async fn send_stale_button(bot: &Bot, chat_id: ChatId) -> HandlerResult {
    bot.send_message(chat_id, super::texts::STALE_BUTTON)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset, NaiveTime};

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
        terms.deadline_at = timezone.parse_deadline("20.08.2027 23:59");
        terms.period_days = 365;
        let deadline = terms.deadline_at.unwrap();
        let policy = LatePolicy::new(365, 1, 1).unwrap();
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
                let periods = u16::try_from(policy.periods_until(deadline, at)).unwrap();
                LatePolicy::new(365, 1, periods)
                    .unwrap()
                    .validate_horizon(deadline)
                    .unwrap();
                dates.push(date);
            }
        }
        assert_eq!(dates.len(), 3);
        assert_eq!(
            dates[0],
            timezone.local_date(deadline) + chrono::Days::new(7)
        );
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
        form.calendar_month = Some(timezone.local_date(created_at));
        let (_, keyboard) = terms_form_view(&form, &timezone);
        let json = serde_json::to_string(&keyboard).unwrap();
        assert!(!json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-20")));
        assert!(json.contains(&format!("{CALENDAR_PICK_PREFIX}2026-08-21")));
    }

    #[test]
    fn a_hundred_periods_still_produce_a_short_two_boundary_preview() {
        let form = TermsForm::new(scored_terms(), TermsStep::Summary);
        let (text, keyboard) = terms_form_view(&form, &timezone());
        assert!(text.contains(&texts::score_preview(9, 0)));
        assert!(text.chars().count() < 1000);
        assert_eq!(keyboard.inline_keyboard.last().unwrap().len(), 1);
        assert_eq!(
            keyboard.inline_keyboard.last().unwrap()[0].text,
            texts::BUTTON_CANCEL_CREATION
        );
    }

    #[test]
    fn calendar_return_and_previous_step_have_distinct_labels_and_actions() {
        let mut form = TermsForm::new(scored_terms(), TermsStep::Deadline);
        form.calendar_month = Some(timezone().today());
        let (_, keyboard) = terms_form_view(&form, &timezone());
        let json = serde_json::to_value(keyboard).unwrap();
        let rows = json["inline_keyboard"].as_array().unwrap();
        assert_eq!(rows[rows.len() - 2][0]["text"], texts::BUTTON_DATE_OPTIONS);
        assert_eq!(rows[rows.len() - 2][0]["callback_data"], DEADLINE_OPTIONS);
        assert_eq!(rows.last().unwrap()[0]["text"], texts::BUTTON_BACK);
        assert_eq!(rows.last().unwrap()[0]["callback_data"], TERMS_BACK);
    }

    fn timezone() -> BotTimezone {
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
    }

    #[test]
    fn long_messages_are_split_without_losing_content() {
        let source = "я".repeat(MESSAGE_LIMIT * 2 + 1);
        let chunks = message_chunks(source.clone());

        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= MESSAGE_LIMIT)
        );
        assert_eq!(chunks.concat(), source);
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

        assert_eq!(deadline_left(past, Utc::now()), "дедлайн прошёл");
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
