use chrono::{DateTime, Utc};
use teloxide::types::InlineKeyboardMarkup;

use super::keyboards::{makeup_form_keyboard, terms_form_keyboard};
use crate::bot::assignment_form::{CompleteTerms, TermsForm, TermsStep, TermsTarget};
use crate::bot::form::InputMode;
use crate::bot::makeup_form::{MakeupForm, MakeupStep};
use crate::bot::{texts, timezone::BotTimezone};
use crate::domain::{
    Assignment, AssignmentStatus, DeductedFrom, Grading, LatePolicy, MakeupCredit, MakeupOffer,
    MaxScore,
};
use crate::telegram::text::truncate;

const MAKEUP_TITLES_SHOWN: usize = 10;

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
                if terms.deducted_from.is_some() {
                    texts::NO_PENALTY_UNTIL
                } else {
                    texts::SUBMIT_UNTIL
                },
                timezone.format(deadline),
            )
        })
        .unwrap_or_default();
    let text = match form.step {
        TermsStep::Makeup => texts::makeup_terms_question(terms.grading()),
        TermsStep::Policy => format!(
            "{}\n\n{}",
            texts::LATE_POLICY_QUESTION,
            texts::DEDUCTED_FROM_HINT
        ),
        TermsStep::Deadline | TermsStep::DurationDate => {
            let question = if form.step == TermsStep::DurationDate {
                texts::DURATION_DATE_QUESTION
            } else if terms.deducted_from.is_some() {
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
            let policy = terms.preview_policy(1)?;
            format!(
                "{context}{}\n{}",
                texts::penalty_rule(policy.period_days(), policy.penalty_per_period()),
                texts::periods_question(terms.period_days),
            )
        }
        TermsStep::DurationConfirm { selected, periods } => {
            let closing = terms
                .preview_policy(periods)?
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
    let mut text = texts::TERMS_CONFIRM.to_owned();
    if let TermsTarget::Create(draft) = &terms.target
        && let Some(codes) = draft.groups.listed()
    {
        text.push_str(&format!("\n{}", texts::groups_line(&codes)));
    }
    text.push_str(&format!(
        "\n{}",
        terms_lines(terms.grading(), deadline, policy, closing, timezone)
    ));
    if terms.makeup_enabled {
        text.push_str(&format!(
            "\n{}",
            texts::makeup_terms_line(terms.makeup, terms.grading())
        ));
    }
    if let (Some(policy), Grading::Score { max_score }) = (policy, terms.grading()) {
        text.push_str(&format!(
            "\n{}",
            penalty_schedule(deadline, policy, closing, max_score, timezone)
        ));
    }
    if let Some(previous) = form.previous_closes_at
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
    let free_until = timezone.format_until(deadline_at);
    let mut lines = match policy.deducted_from() {
        DeductedFrom::WorkScore => vec![
            texts::SCHEDULE_HEADER.to_owned(),
            texts::schedule_free_row(&free_until, max),
        ],
        DeductedFrom::CourseTotal => vec![
            texts::SCHEDULE_TOTAL_HEADER.to_owned(),
            texts::schedule_free_total_row(&free_until),
        ],
    };
    let mut previous = 0;
    for periods in schedule_periods(policy, max_score) {
        let Ok(until) = policy.period_end(deadline_at, periods) else {
            break;
        };
        if periods > previous + 1 {
            lines.push(texts::schedule_skipped(u32::from(periods - previous - 1)));
        }
        let until = timezone.format_until(until);
        lines.push(match policy.deducted_from() {
            DeductedFrom::WorkScore => {
                let periods = u32::from(periods);
                let score = policy.possible_score(max_score, periods);
                texts::schedule_penalty_row(&until, policy.penalty_for(periods), score, max)
            }
            DeductedFrom::CourseTotal => {
                texts::schedule_total_row(&until, policy.penalty_for(u32::from(periods)))
            }
        });
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
    student_grading_lines(assignment, None, now, timezone)
}

pub(crate) fn student_grading_lines(
    assignment: &Assignment,
    makeup: Option<MakeupOffer>,
    now: DateTime<Utc>,
    timezone: &BotTimezone,
) -> String {
    let closes_at = assignment.closes_at();
    let archived = assignment.status() == AssignmentStatus::Archived;
    let open = !archived && now < closes_at;
    let terms = match (assignment.late_policy(), assignment.grading()) {
        (Some(policy), Grading::Score { max_score }) if open => format!(
            "{} {}\n{}\n{}",
            texts::MAX_SCORE_LABEL,
            texts::points(max_score.get()),
            texts::deducted_from_line(policy.deducted_from()),
            penalty_schedule(
                assignment.deadline_at(),
                policy,
                closes_at,
                max_score,
                timezone
            )
        ),
        _ => terms_lines(
            assignment.grading(),
            assignment.deadline_at(),
            assignment.late_policy(),
            closes_at,
            timezone,
        ),
    };
    let mut lines = format!("\n{terms}");
    match makeup {
        _ if archived => {
            lines.push_str(&format!("\n{}", texts::ARCHIVED_CLOSED));
        }
        Some(makeup) if now >= closes_at => {
            lines.push_str(&format!(
                "\n{}",
                student_makeup_lines(makeup, assignment.grading(), timezone)
            ));
        }
        _ if now >= closes_at => {
            lines.push_str(&format!(
                "\n{} {}.",
                texts::CLOSED_ON,
                timezone.format_closed_since(closes_at)
            ));
        }
        _ => {}
    }
    if let Some(penalty) = assignment.late_penalty_at(now)
        && open
    {
        lines.push_str(&format!(
            "\n{} −{}",
            texts::LATE_PENALTY_LABEL,
            penalty.points()
        ));
    }
    lines
}

pub(crate) fn student_makeup_lines(
    makeup: MakeupOffer,
    grading: Grading,
    timezone: &BotTimezone,
) -> String {
    let offer = texts::makeup_offer(
        &timezone.format(makeup.until),
        &makeup_credit_terms(makeup.credit, grading),
    );
    let balance = makeup.balance;
    let attempts = match balance.remaining() {
        0 => texts::MAKEUP_ATTEMPTS_EXHAUSTED.to_owned(),
        remaining => texts::card_makeup_attempts(remaining, balance.allowed()),
    };
    format!("{offer}\n{attempts}")
}

pub(crate) fn makeup_credit_terms(credit: Option<MakeupCredit>, grading: Grading) -> String {
    texts::makeup_credit_text(credit.map(MakeupCredit::get).zip(grading.max_score()))
}

pub(crate) fn makeup_form_view(
    form: &MakeupForm,
    timezone: &BotTimezone,
    now: DateTime<Utc>,
) -> Option<(String, InlineKeyboardMarkup)> {
    let text = match form.step {
        MakeupStep::Date => {
            let current = form
                .until
                .map(|until| texts::makeup_until_line(&timezone.format(until)));
            let ended = form
                .ended()
                .map(|makeup| texts::makeup_ended_line(&timezone.format(makeup.until())));
            let context = current
                .or(ended)
                .map(|line| format!("{line}.\n"))
                .unwrap_or_default();
            format!(
                "{context}{}\n{} {} {}.\n{}",
                texts::MAKEUP_DATE_QUESTION,
                texts::WHOLE_DAY_HINT,
                texts::TIMEZONE_LABEL,
                timezone.label(),
                texts::DATE_FORMAT,
            )
        }
        MakeupStep::Attempts => texts::makeup_attempts_question(),
        MakeupStep::Scale(_) => {
            let scale = form.current_scale()?;
            let titles: Vec<String> = scale
                .assignments
                .iter()
                .take(MAKEUP_TITLES_SHOWN)
                .map(|(_, title)| button_label(title))
                .collect();
            let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
            texts::makeup_scale_question(
                scale.grading,
                scale.assignments.len(),
                &titles,
                scale.assignments.len().saturating_sub(MAKEUP_TITLES_SHOWN),
            )
        }
        MakeupStep::Summary => makeup_summary(form, timezone)?,
    };
    let keyboard = match form.mode {
        InputMode::Manual => InlineKeyboardMarkup::default(),
        InputMode::Buttons | InputMode::Calendar(_) => makeup_form_keyboard(form, timezone, now)?,
    };
    Some((text, keyboard))
}

fn makeup_summary(form: &MakeupForm, timezone: &BotTimezone) -> Option<String> {
    let mut lines = vec![texts::MAKEUP_SUMMARY_HEADER.to_owned()];
    if form.disabling {
        lines.push(texts::MAKEUP_WILL_BE_DISABLED.to_owned());
    } else {
        lines.push(format!(
            "{}.",
            texts::makeup_until_line(&timezone.format(form.until?))
        ));
        lines.push(texts::makeup_attempts_line(form.attempts?));
    }
    for scale in &form.scales {
        let answer = texts::makeup_answer(scale.answer?, scale.grading);
        lines.push(texts::makeup_scale_answer(
            scale.grading,
            scale.assignments.len(),
            &answer,
        ));
    }
    if !form.active && !form.disabling {
        lines.push(texts::MAKEUP_NEW_POOL.to_owned());
    }
    Some(lines.join("\n"))
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
            "{scale}\n{} {}.\n{}\n{}\n{} {}.\n{} {}.",
            texts::NO_PENALTY_UNTIL,
            timezone.format(deadline_at),
            texts::penalty_rule(policy.period_days(), policy.penalty_per_period()),
            texts::deducted_from_line(policy.deducted_from()),
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
    truncate(label, 60)
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::app::MakeupQuestion;
    use crate::bot::assignment_form::AssignmentTerms;
    use crate::bot::callbacks::TermsAction;
    use crate::domain::DeductedFrom;
    use crate::test_support::{assignment, course_for, student_group, t0, teacher, vladivostok};

    #[test]
    fn summary_warns_at_a_past_deadline_according_to_the_selected_policy() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let timezone = vladivostok();
        for allow_late in [false, true] {
            let mut builder = assignment(&course, &teacher).scored(5);
            if allow_late {
                builder = builder.late_policy(7, 1, 4, DeductedFrom::WorkScore);
            }
            let assignment = builder.build();
            let form = TermsForm::new(
                AssignmentTerms::editing(&assignment, 0, MakeupQuestion::default()),
                TermsStep::Summary,
            );
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
            .late_policy(7, 2, 4, DeductedFrom::WorkScore)
            .build();
        let timezone = vladivostok();
        let mut form = TermsForm::new(
            AssignmentTerms::editing(&assignment, 0, MakeupQuestion::default()),
            TermsStep::Summary,
        );
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
    fn the_summary_its_policy_button_and_the_student_card_name_where_the_penalty_is_deducted_from()
    {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let timezone = vladivostok();
        for (deducted_from, button) in [
            (DeductedFrom::WorkScore, texts::BUTTON_POLICY_WORK_SCORE),
            (DeductedFrom::CourseTotal, texts::BUTTON_POLICY_COURSE_TOTAL),
        ] {
            let assignment = assignment(&course, &teacher)
                .scored(10)
                .late_policy(7, 2, 4, deducted_from)
                .build();
            let form = TermsForm::new(
                AssignmentTerms::editing(&assignment, 0, MakeupQuestion::default()),
                TermsStep::Summary,
            );

            let (summary, keyboard) = terms_form_view(&form, &timezone, t0()).unwrap();
            let card = grading_lines(&assignment, t0(), &timezone);

            let line = texts::deducted_from_line(deducted_from);
            assert!(summary.contains(&line), "{summary}");
            assert!(card.contains(&line), "{card}");
            assert!(
                keyboard
                    .inline_keyboard
                    .iter()
                    .flatten()
                    .any(|key| key.text == button),
                "{deducted_from:?}"
            );
        }
    }

    #[test]
    fn the_student_card_shows_the_current_penalty_only_while_submissions_are_open() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let assignment = assignment(&course, &teacher)
            .scored(5)
            .late_policy(7, 1, 4, DeductedFrom::WorkScore)
            .build();
        let timezone = vladivostok();
        let late = grading_lines(
            &assignment,
            assignment.deadline_at() + Duration::days(1),
            &timezone,
        );
        assert!(
            late.contains(&format!("{} −1", texts::LATE_PENALTY_LABEL)),
            "{late}"
        );
        let schedule = penalty_schedule(
            assignment.deadline_at(),
            assignment.late_policy().unwrap(),
            assignment.closes_at(),
            MaxScore::new(5).unwrap(),
            &timezone,
        );
        assert!(late.contains(&schedule), "{late}");

        let closed = grading_lines(&assignment, assignment.closes_at(), &timezone);
        assert!(closed.contains(texts::CLOSED_ON), "{closed}");
        assert!(!closed.contains(texts::LATE_PENALTY_LABEL), "{closed}");
    }

    #[test]
    fn an_archived_card_shows_no_schedule_or_current_penalty_before_the_closing() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let mut assignment = assignment(&course, &teacher)
            .scored(5)
            .late_policy(7, 1, 4, DeductedFrom::WorkScore)
            .published()
            .build();
        assignment.archive(&course, &teacher).unwrap();
        let timezone = vladivostok();
        let schedule = penalty_schedule(
            assignment.deadline_at(),
            assignment.late_policy().unwrap(),
            assignment.closes_at(),
            MaxScore::new(5).unwrap(),
            &timezone,
        );

        let card = grading_lines(
            &assignment,
            assignment.deadline_at() + Duration::days(1),
            &timezone,
        );

        assert!(!card.contains(&schedule), "{card}");
        assert!(!card.contains(texts::LATE_PENALTY_LABEL), "{card}");
        assert!(card.contains(texts::ARCHIVED_CLOSED), "{card}");
    }

    #[test]
    fn an_ended_makeup_is_named_at_the_date_step_and_announced_as_a_new_pool() {
        use crate::bot::form::Form;
        use crate::bot::makeup_form::MakeupForm;
        use crate::domain::{CourseMakeup, TeachingCourseId};

        let timezone = vladivostok();
        let ended = CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap();
        let now = ended.until() + Duration::days(1);
        let mut form = MakeupForm::new(TeachingCourseId::new(), Some(ended), Vec::new(), now);

        let (date_step, _) = makeup_form_view(&form, &timezone, now).unwrap();
        assert!(
            date_step.contains(&texts::makeup_ended_line(&timezone.format(ended.until()))),
            "{date_step}"
        );
        form.apply_date(now + Duration::days(30), now).unwrap();
        form.apply_attempts(2).unwrap();
        let (summary, _) = makeup_form_view(&form, &timezone, now).unwrap();
        assert!(summary.contains(texts::MAKEUP_NEW_POOL), "{summary}");
    }

    #[test]
    fn the_summary_names_the_makeup_terms_only_on_a_course_with_makeup() {
        use crate::domain::{Grading, MakeupTerms};

        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let timezone = vladivostok();
        let scored = assignment(&course, &teacher).scored(10).build();
        let pass_fail = assignment(&course, &teacher).build();
        let six = MakeupTerms::scored(6, Grading::scored(10).unwrap()).unwrap();
        let summary = |assignment: &Assignment, makeup, enabled| {
            let mut terms = AssignmentTerms::editing(
                assignment,
                0,
                MakeupQuestion {
                    enabled,
                    hint: None,
                },
            );
            terms.makeup = makeup;
            let form = TermsForm::new(terms, TermsStep::Summary);
            terms_form_view(&form, &timezone, t0()).unwrap()
        };

        for (assignment, makeup) in [
            (&scored, Some(six)),
            (&scored, Some(MakeupTerms::NotOffered)),
            (&pass_fail, Some(MakeupTerms::PassFail)),
            (&scored, None),
        ] {
            let line = texts::makeup_terms_line(makeup, assignment.grading());
            let (text, keyboard) = summary(assignment, makeup, true);
            assert!(text.contains(&line), "{text}");
            assert!(
                keyboard
                    .inline_keyboard
                    .iter()
                    .flatten()
                    .any(|button| button.text == line),
                "{line}"
            );
        }
        assert!(
            texts::makeup_terms_line(Some(six), scored.grading()).contains("6 из 10"),
            "the credit is named out of the scale"
        );
        let (text, _) = summary(&scored, Some(six), false);
        assert!(!text.contains(texts::MAKEUP_MARK), "{text}");
    }

    #[test]
    fn a_closed_card_shows_the_makeup_instead_of_the_closing_or_says_the_pool_is_empty() {
        use crate::domain::{CourseMakeup, MakeupTerms};

        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let timezone = vladivostok();
        let work = assignment(&course, &teacher)
            .scored(10)
            .makeup_credit(6)
            .build();
        let closed = work.closes_at() + Duration::days(1);
        let window = CourseMakeup::open(closed + Duration::days(30), 3, work.closes_at()).unwrap();
        let offer = |used| MakeupOffer {
            until: window.until(),
            credit: work.makeup().and_then(MakeupTerms::credit),
            balance: window.balance(0, used),
        };
        let offer_line = texts::makeup_offer(
            &timezone.format(window.until()),
            &texts::makeup_credit_text(Some((6, 10))),
        );

        let available = student_grading_lines(&work, Some(offer(1)), closed, &timezone);
        assert!(available.contains(&offer_line), "{available}");
        assert!(
            available.contains(&texts::card_makeup_attempts(2, 3)),
            "{available}"
        );
        assert!(!available.contains(texts::CLOSED_ON), "{available}");

        let exhausted = student_grading_lines(&work, Some(offer(3)), closed, &timezone);
        assert!(exhausted.contains(&offer_line), "{exhausted}");
        assert!(
            exhausted.contains(texts::MAKEUP_ATTEMPTS_EXHAUSTED),
            "{exhausted}"
        );

        let none = student_grading_lines(&work, None, closed, &timezone);
        assert!(none.contains(texts::CLOSED_ON), "{none}");
        assert!(!none.contains(texts::MAKEUP_MARK), "{none}");
    }
}
