use chrono::{DateTime, Utc};

use crate::bot::state::{TermsForm, TermsStep, TermsTarget};
use crate::bot::{texts, timezone::BotTimezone};
use crate::domain::{
    Assignment, AssignmentStatus, Grade, Grading, LatePolicy, MaxScore, Role, SubmissionStatus,
};
use crate::telegram::text::truncate;
use teloxide::types::InlineKeyboardMarkup;

pub(in crate::bot) fn terms_form_view(
    form: &TermsForm,
    timezone: &BotTimezone,
) -> (String, InlineKeyboardMarkup) {
    let terms = &form.terms;
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
                timezone.format(deadline),
            )
        })
        .unwrap_or_default();
    let text = match form.step {
        TermsStep::Policy => texts::LATE_POLICY_QUESTION.to_owned(),
        TermsStep::Deadline | TermsStep::DurationDate => {
            let question = if form.step == TermsStep::DurationDate {
                texts::DURATION_DATE_QUESTION
            } else if terms.allow_late {
                texts::NO_PENALTY_QUESTION
            } else {
                texts::DEADLINE_QUESTION
            };
            format!(
                "{context}{question}\nВремя по умолчанию — {} ({}). Можно прислать дату сообщением.\n{}",
                timezone.deadline_time().format("%H:%M"),
                timezone.suffix(),
                texts::DATE_FORMAT,
            )
        }
        TermsStep::Penalty => format!(
            "{context}{}\nРазмер штрафа в баллах за каждые начавшиеся {} дн. Пришлите число от 1 до 100 сообщением.",
            texts::penalty_question(terms.period_days),
            terms.period_days,
        ),
        TermsStep::Period => format!("{context}{}", texts::PERIOD_PROMPT),
        TermsStep::Periods => {
            let policy = terms
                .preview_policy(1)
                .expect("validated period and penalty");
            format!(
                "{context}{}\n{}",
                texts::penalty_rule(policy.period_days(), policy.penalty()),
                texts::periods_question(terms.period_days),
            )
        }
        TermsStep::DurationConfirm { selected, periods } => {
            let policy = terms.preview_policy(periods).expect("validated duration");
            let closing = policy
                .closes_at(terms.deadline_at.expect("duration follows deadline"))
                .expect("validated duration");
            format!(
                "{} — это {}.\n{} {} включительно. Применить?",
                timezone.format(selected),
                texts::lateness_duration(terms.period_days, periods),
                texts::SUBMISSIONS_CLOSE_LABEL,
                timezone.format(closing),
            )
        }
        TermsStep::Summary => terms_summary(form, timezone),
    };
    (text, super::keyboards::terms_form_keyboard(form, timezone))
}

fn terms_summary(form: &TermsForm, timezone: &BotTimezone) -> String {
    let terms = &form.terms;
    let deadline = terms.deadline_at.expect("summary requires deadline");
    let policy = terms
        .committed_policy()
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
            penalty_schedule(deadline, policy, max_score, timezone)
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
    text
}

const SCHEDULE_ROWS: u16 = 5;

fn schedule_periods(policy: LatePolicy, max_score: MaxScore) -> Vec<u16> {
    let last = (1..=policy.max_periods())
        .find(|&periods| policy.possible_score(max_score, u32::from(periods)) == 0)
        .unwrap_or_else(|| policy.max_periods());
    let head = last.min(SCHEDULE_ROWS);
    let mut periods: Vec<_> = (1..=head).collect();
    if last > head {
        periods.push(last);
    }
    periods
}

pub(super) fn penalty_schedule(
    deadline_at: DateTime<Utc>,
    policy: LatePolicy,
    max_score: MaxScore,
    timezone: &BotTimezone,
) -> String {
    let max = max_score.get();
    let mut lines = vec![
        texts::SCHEDULE_HEADER.to_owned(),
        texts::schedule_free_row(&timezone.format_date_time(deadline_at), max),
    ];
    let mut previous = 0;
    for periods in schedule_periods(policy, max_score) {
        if periods > previous + 1 {
            lines.push(texts::schedule_skipped(u32::from(periods - previous - 1)));
        }
        let score = policy.possible_score(max_score, u32::from(periods));
        let until = policy
            .period_end(deadline_at, periods)
            .expect("validated horizon");
        lines.push(texts::schedule_penalty_row(
            &timezone.format_date_time(until),
            max - score,
            score,
            max,
        ));
        previous = periods;
    }
    let closes_at = policy.closes_at(deadline_at).expect("validated horizon");
    lines.push(texts::schedule_closed(
        &timezone.format_date_time(closes_at),
    ));
    lines.join("\n")
}

pub(in crate::bot) fn format_file_size(size_bytes: u64) -> String {
    if size_bytes < 1024 {
        format!("{size_bytes} Б")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} КиБ", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.1} МиБ", size_bytes as f64 / (1024.0 * 1024.0))
    }
}

pub(in crate::bot) fn deadline_left(deadline: DateTime<Utc>, now: DateTime<Utc>) -> String {
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

pub(in crate::bot) fn grading_lines(
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
            timezone,
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
        lines.push_str(&format!("\n{} −{penalty}", texts::LATE_PENALTY_LABEL));
    }
    lines
}

pub(in crate::bot) fn terms_lines(
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

pub(in crate::bot) fn grade_line(grade: Grade) -> String {
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

pub(in crate::bot) fn submission_status(status: SubmissionStatus) -> &'static str {
    match status {
        SubmissionStatus::PendingReview => "⏳ на проверке",
        SubmissionStatus::Accepted => "✅ принята",
        SubmissionStatus::Rejected => "❌ отклонена",
    }
}

pub(in crate::bot) fn assignment_status(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️ черновик",
        AssignmentStatus::Published => "🟢 опубликовано",
        AssignmentStatus::Archived => "📦 архив",
    }
}

pub(super) fn assignment_status_icon(status: AssignmentStatus) -> &'static str {
    match status {
        AssignmentStatus::Draft => "⚪️",
        AssignmentStatus::Published => "🟢",
        AssignmentStatus::Archived => "📦",
    }
}

pub(in crate::bot) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Student { .. } => "🎓 студент",
        Role::Teacher => "👨‍🏫 преподаватель",
        Role::Admin => "🛠 администратор",
    }
}

pub(in crate::bot) fn button_label(label: &str) -> String {
    truncate(label, 60, true)
}
