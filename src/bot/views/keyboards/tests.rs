use chrono::DateTime;

use super::super::render::{penalty_schedule, terms_form_view};
use super::*;
use crate::bot::assignment_form::{AssignmentText, DraftAssignment, IssuedGroups};
use crate::domain::{Grading, LatePolicy, MaxScore};
use crate::test_support::{t0, vladivostok};

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
            let teloxide::types::InlineKeyboardButtonKind::CallbackData(data) = &button.kind else {
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let (text, keyboard) = terms_form_view(&form, &vladivostok(), t0()).unwrap();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
    let timezone = vladivostok();
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
}

#[test]
fn a_schedule_to_the_course_total_runs_to_the_period_limit() {
    let timezone = vladivostok();
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
    form.mode = InputMode::Calendar(vladivostok().local_date(t0()));
    let (_, keyboard) = terms_form_view(&form, &vladivostok(), t0()).unwrap();
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
        let keyboard = makeup_form_keyboard(form, &vladivostok(), t0()).unwrap();
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
    let timezone = vladivostok();
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
    let json = serde_json::to_value(deadline_options_keyboard(&vladivostok(), t0())).unwrap();
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
        !buttons(makeup_answer_keyboard(ten, None)).contains(&(texts::makeup_hint_button(6), six))
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
