use chrono::{DateTime, Utc};
use teloxide::types::InlineKeyboardMarkup;

use super::keyboards::terms_form_keyboard;
use crate::bot::assignment_form::{CompleteTerms, TermsForm, TermsStep, TermsTarget};
use crate::bot::{texts, timezone::BotTimezone};
use crate::domain::{Assignment, Grading, LatePolicy, MaxScore};
use crate::telegram::text::truncate;

/// `None` означает, что в форме нет значений, без которых её шаг не показать.
pub(crate) fn terms_form_view(
    form: &TermsForm,
    timezone: &BotTimezone,
    now: DateTime<Utc>,
) -> Option<(String, InlineKeyboardMarkup)> {
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
                "{context}{question}\n{} {} {}.\n{}",
                texts::WHOLE_DAY_HINT,
                texts::TIMEZONE_LABEL,
                timezone.label(),
                texts::DATE_FORMAT,
            )
        }
        TermsStep::Penalty => format!(
            "{context}{}",
            texts::penalty_step(
                &texts::penalty_question(terms.period_days),
                terms.period_days
            )
        ),
        TermsStep::Period => format!("{context}{}", texts::period_prompt()),
        TermsStep::Periods => {
            let policy = terms.preview_policy(1).ok()?;
            format!(
                "{context}{}\n{}",
                texts::penalty_rule(policy.period_days(), policy.penalty()),
                texts::periods_question(terms.period_days),
            )
        }
        TermsStep::DurationConfirm { selected, periods } => {
            let closing = terms
                .preview_policy(periods)
                .ok()?
                .closes_at(terms.deadline_at?)
                .ok()?;
            texts::duration_confirmation(
                &timezone.format(selected),
                &texts::lateness_duration(terms.period_days, periods),
                &timezone.format(closing),
            )
        }
        TermsStep::Summary => terms_summary(form, terms.complete()?, timezone, now),
    };
    Some((text, terms_form_keyboard(form, timezone, now)?))
}

fn terms_summary(
    form: &TermsForm,
    complete: CompleteTerms,
    timezone: &BotTimezone,
    now: DateTime<Utc>,
) -> String {
    let terms = &form.terms;
    let CompleteTerms {
        deadline_at: deadline,
        late_policy: policy,
        closes_at: closing,
    } = complete;
    let mut text = format!(
        "{}\n{}",
        texts::TERMS_CONFIRM,
        terms_lines(terms.grading(), deadline, policy, closing, timezone)
    );
    if let (Some(policy), Grading::Score { max_score }) = (policy, terms.grading()) {
        text.push_str(&format!(
            "\n{}",
            penalty_schedule(deadline, policy, closing, max_score, timezone)
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
    if matches!(terms.target, TermsTarget::Edit { .. }) && deadline <= now {
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
    closes_at: DateTime<Utc>,
    max_score: MaxScore,
    timezone: &BotTimezone,
) -> String {
    let max = max_score.get();
    let mut lines = vec![
        texts::SCHEDULE_HEADER.to_owned(),
        texts::schedule_free_row(&timezone.format_until(deadline_at), max),
    ];
    let mut previous = 0;
    for periods in schedule_periods(policy, max_score) {
        let Ok(until) = policy.period_end(deadline_at, periods) else {
            break;
        };
        if periods > previous + 1 {
            lines.push(texts::schedule_skipped(u32::from(periods - previous - 1)));
        }
        let score = policy.possible_score(max_score, u32::from(periods));
        lines.push(texts::schedule_penalty_row(
            &timezone.format_until(until),
            max - score,
            score,
            max,
        ));
        previous = periods;
    }
    lines.push(texts::schedule_closed(&timezone.format_since(closes_at)));
    lines.join("\n")
}

pub(crate) fn grading_lines(
    assignment: &Assignment,
    now: DateTime<Utc>,
    timezone: &BotTimezone,
) -> String {
    let closes_at = assignment.closes_at();
    let mut lines = format!(
        "\n{}",
        terms_lines(
            assignment.grading(),
            assignment.deadline_at(),
            assignment.late_policy(),
            closes_at,
            timezone,
        )
    );
    if now >= closes_at {
        lines.push_str(&format!(
            "\n{} {}.",
            texts::CLOSED_ON,
            timezone.format_closed_since(closes_at)
        ));
    }
    let penalty = assignment.penalty_at(now);
    if penalty > 0 && now < closes_at {
        lines.push_str(&format!("\n{} −{penalty}", texts::LATE_PENALTY_LABEL));
    }
    lines
}

fn terms_lines(
    grading: Grading,
    deadline_at: DateTime<Utc>,
    late_policy: Option<LatePolicy>,
    closes_at: DateTime<Utc>,
    timezone: &BotTimezone,
) -> String {
    let scale = match grading.max_score() {
        Some(max) => format!("{} {}", texts::MAX_SCORE_LABEL, texts::points(max)),
        None => texts::PASS_FAIL_LABEL.to_owned(),
    };
    match late_policy {
        Some(policy) => format!(
            "{scale}\n{} {}.\n{}\n{} {}.\n{} {}.",
            texts::NO_PENALTY_UNTIL,
            timezone.format(deadline_at),
            texts::penalty_rule(policy.period_days(), policy.penalty()),
            texts::LATENESS_LABEL,
            texts::lateness_duration(policy.period_days(), policy.max_periods()),
            texts::SUBMISSIONS_CLOSE_LABEL,
            timezone.format(closes_at)
        ),
        None => format!(
            "{scale}\n{} {}. {}",
            texts::SUBMIT_UNTIL,
            timezone.format(deadline_at),
            texts::STRICT_AFTER_DEADLINE
        ),
    }
}

pub(crate) fn list_text(header: &str, entries: impl IntoIterator<Item = String>) -> String {
    std::iter::once(header.to_owned())
        .chain(entries)
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn button_label(label: &str) -> String {
    truncate(label, 60, true)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use chrono_tz::Tz;

    use super::*;
    use crate::bot::assignment_form::AssignmentTerms;
    use crate::bot::callbacks::TermsAction;
    use crate::test_support::{assignment, course_for, student_group, t0, teacher};

    #[test]
    fn summary_warns_at_a_past_deadline_according_to_the_selected_policy() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let timezone = BotTimezone::new(Tz::Asia__Vladivostok);
        for allow_late in [false, true] {
            let mut builder = assignment(&course, &teacher).scored(5);
            if allow_late {
                builder = builder.late_policy(7, 1, 4);
            }
            let assignment = builder.build();
            let form = TermsForm::new(AssignmentTerms::editing(&assignment, 0), TermsStep::Summary);
            let before = terms_form_view(
                &form,
                &timezone,
                assignment.deadline_at() - Duration::nanoseconds(1),
            )
            .unwrap()
            .0;
            assert!(!before.contains(texts::PAST_STRICT_WARNING));
            assert!(!before.contains(texts::PAST_PENALTY_WARNING));

            let at_deadline = terms_form_view(&form, &timezone, assignment.deadline_at())
                .unwrap()
                .0;
            assert_eq!(
                at_deadline.contains(texts::PAST_PENALTY_WARNING),
                allow_late
            );
            assert_eq!(
                at_deadline.contains(texts::PAST_STRICT_WARNING),
                !allow_late
            );
        }
    }

    #[test]
    fn summary_previews_the_original_and_new_computed_closing() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let assignment = assignment(&course, &teacher)
            .scored(5)
            .late_policy(7, 2, 4)
            .build();
        let timezone = BotTimezone::new(Tz::Asia__Vladivostok);
        let mut form = TermsForm::new(AssignmentTerms::editing(&assignment, 0), TermsStep::Summary);
        assert!(form.edit_summary(TermsAction::Period));
        form.apply_number(10).unwrap();

        let (text, _) = terms_form_view(&form, &timezone, t0()).unwrap();

        assert!(
            text.contains(&texts::closing_change(
                &timezone.format(assignment.closes_at()),
                &timezone.format(assignment.deadline_at() + Duration::days(40)),
            )),
            "{text}"
        );
    }

    #[test]
    fn the_student_card_shows_the_current_penalty_only_while_submissions_are_open() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let assignment = assignment(&course, &teacher)
            .scored(5)
            .late_policy(7, 1, 4)
            .build();
        let timezone = BotTimezone::new(Tz::Asia__Vladivostok);
        let late = grading_lines(
            &assignment,
            assignment.deadline_at() + Duration::days(1),
            &timezone,
        );
        assert!(
            late.contains(&format!("{} −1", texts::LATE_PENALTY_LABEL)),
            "{late}"
        );
        assert!(late.contains(texts::SUBMISSIONS_CLOSE_LABEL), "{late}");

        let closed = grading_lines(&assignment, assignment.closes_at(), &timezone);
        assert!(closed.contains(texts::CLOSED_ON), "{closed}");
        assert!(!closed.contains(texts::LATE_PENALTY_LABEL), "{closed}");
    }
}
