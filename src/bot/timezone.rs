use chrono::{
    DateTime, Days, LocalResult, Months, NaiveDate, NaiveTime, Offset, TimeDelta, TimeZone, Utc,
};
use chrono_tz::Tz;

/// Границы сроков исключительные, как у интервалов RFC 5545: «до конца дня 21.09» хранится как
/// 22.09 00:00 в поясе установки, и в этот момент приём уже закрыт.
#[derive(Clone, Copy, Debug)]
pub struct BotTimezone {
    tz: Tz,
}

const DATE: &str = "%d.%m.%Y";
const DATE_TIME: &str = "%d.%m.%Y %H:%M";
const SHORT_DATE: &str = "%d.%m";
const SHORT_DATE_TIME: &str = "%d.%m %H:%M";

impl BotTimezone {
    pub fn new(tz: Tz) -> Self {
        Self { tz }
    }

    pub fn label(&self) -> String {
        self.label_at(Utc::now())
    }

    fn label_at(&self, instant: DateTime<Utc>) -> String {
        let city = self
            .tz
            .name()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .replace('_', " ");
        let total_minutes = instant
            .with_timezone(&self.tz)
            .offset()
            .fix()
            .local_minus_utc()
            / 60;
        let sign = if total_minutes < 0 { '-' } else { '+' };
        let hours = total_minutes.abs() / 60;
        let minutes = total_minutes.abs() % 60;

        if minutes == 0 {
            format!("{city} (UTC{sign}{hours})")
        } else {
            format!("{city} (UTC{sign}{hours}:{minutes:02})")
        }
    }

    pub fn today(&self) -> NaiveDate {
        self.local_date(Utc::now())
    }

    pub fn local_date(&self, instant: DateTime<Utc>) -> NaiveDate {
        instant.with_timezone(&self.tz).date_naive()
    }

    pub fn first_date_ending_after(&self, instant: DateTime<Utc>) -> NaiveDate {
        self.local_date(instant)
    }

    pub fn format(&self, instant: DateTime<Utc>) -> String {
        format!("до {}", self.format_until(instant))
    }

    pub fn format_closed_since(&self, instant: DateTime<Utc>) -> String {
        format!("с {}", self.format_since(instant))
    }

    pub fn format_until(&self, instant: DateTime<Utc>) -> String {
        self.end_label(instant, DATE, DATE_TIME)
    }

    pub fn format_until_short(&self, instant: DateTime<Utc>) -> String {
        self.end_label(instant, SHORT_DATE, SHORT_DATE_TIME)
    }

    pub fn format_since(&self, instant: DateTime<Utc>) -> String {
        let local = instant.with_timezone(&self.tz);
        if local.time() == NaiveTime::MIN {
            local.format(DATE).to_string()
        } else {
            local.format(DATE_TIME).to_string()
        }
    }

    fn end_label(&self, instant: DateTime<Utc>, date: &str, date_time: &str) -> String {
        let local = instant.with_timezone(&self.tz);
        match local.date_naive().pred_opt() {
            Some(day) if local.time() == NaiveTime::MIN => {
                format!("конца дня {}", day.format(date))
            }
            _ => local.format(date_time).to_string(),
        }
    }

    pub fn format_short(&self, instant: DateTime<Utc>) -> String {
        instant
            .with_timezone(&self.tz)
            .format(SHORT_DATE_TIME)
            .to_string()
    }

    pub fn format_date(&self, date: NaiveDate) -> String {
        date.format(DATE).to_string()
    }

    /// Конец дня - это 00:00 следующего дня
    pub fn end_of(&self, date: NaiveDate) -> Option<DateTime<Utc>> {
        self.instant(date.succ_opt()?, NaiveTime::MIN)
    }

    pub fn in_days(&self, days: u64) -> Option<NaiveDate> {
        self.today().checked_add_days(Days::new(days))
    }

    pub fn in_months(&self, months: u32) -> Option<NaiveDate> {
        self.today().checked_add_months(Months::new(months))
    }

    pub fn parse_deadline(&self, text: &str) -> Option<DateTime<Utc>> {
        let text = text.trim();

        if let Some((date, time)) = text.split_once(' ') {
            let date = NaiveDate::parse_from_str(date.trim(), DATE).ok()?;
            let time = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
            self.instant(date, time)
        } else {
            self.end_of(NaiveDate::parse_from_str(text, DATE).ok()?)
        }
    }

    fn instant(&self, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
        let local = date.and_time(time);
        let resolved = match self.tz.from_local_datetime(&local) {
            LocalResult::Single(instant) | LocalResult::Ambiguous(instant, _) => instant,
            LocalResult::None => {
                let before = self
                    .tz
                    .from_local_datetime(&(local - TimeDelta::hours(1)))
                    .earliest()?;
                let offset = i64::from(before.offset().fix().local_minus_utc());
                return Some(Utc.from_utc_datetime(&(local - TimeDelta::seconds(offset))));
            }
        };
        Some(resolved.with_timezone(&Utc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vladivostok() -> BotTimezone {
        BotTimezone::new(Tz::Asia__Vladivostok)
    }

    fn at(rfc3339: &str) -> DateTime<Utc> {
        rfc3339.parse().unwrap()
    }

    #[test]
    fn a_whole_day_deadline_ends_at_the_next_local_midnight() {
        let timezone = vladivostok();
        let date = NaiveDate::from_ymd_opt(2026, 8, 20).unwrap();

        assert_eq!(
            timezone.end_of(date).unwrap(),
            "2026-08-20T14:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(timezone.parse_deadline("20.08.2026"), timezone.end_of(date));
    }

    #[test]
    fn the_first_selectable_date_is_the_local_day_of_the_instant() {
        let timezone = vladivostok();
        let date = NaiveDate::from_ymd_opt(2026, 8, 21).unwrap();
        let end = timezone.end_of(date).unwrap();

        assert_eq!(
            timezone.first_date_ending_after(end - chrono::Duration::seconds(1)),
            date
        );
        assert_eq!(
            timezone.first_date_ending_after(end),
            date.succ_opt().unwrap()
        );
    }

    #[test]
    fn whole_day_deadlines_are_shown_as_the_end_of_the_day_they_close() {
        let timezone = vladivostok();
        let deadline = timezone.parse_deadline("20.08.2026").unwrap();

        assert_eq!(timezone.format(deadline), "до конца дня 20.08.2026");
        assert_eq!(timezone.format_until(deadline), "конца дня 20.08.2026");
        assert_eq!(timezone.format_until_short(deadline), "конца дня 20.08");
        assert_eq!(timezone.format_since(deadline), "21.08.2026");
        assert_eq!(timezone.format_closed_since(deadline), "с 21.08.2026");
    }

    #[test]
    fn explicit_times_are_read_and_shown_in_the_configured_timezone() {
        let timezone = vladivostok();
        let deadline = timezone.parse_deadline("20.08.2026 15:00").unwrap();

        assert_eq!(
            deadline,
            "2026-08-20T05:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(timezone.format(deadline), "до 20.08.2026 15:00");
        assert_eq!(timezone.format_until_short(deadline), "20.08 15:00");
        assert_eq!(timezone.format_since(deadline), "20.08.2026 15:00");
    }

    #[test]
    fn malformed_deadlines_are_rejected() {
        let timezone = vladivostok();

        assert!(timezone.parse_deadline("20.08.2026 23:59 UTC").is_none());
        assert!(timezone.parse_deadline("31.02.2026 10:00").is_none());
        assert!(timezone.parse_deadline("завтра").is_none());
    }

    #[test]
    fn a_clock_jump_at_midnight_still_closes_the_day() {
        let santiago = BotTimezone::new(Tz::America__Santiago);
        let date = NaiveDate::from_ymd_opt(2025, 9, 6).unwrap();

        assert_eq!(santiago.end_of(date), Some(at("2025-09-07T04:00:00Z")));
        assert_eq!(santiago.parse_deadline("06.09.2025"), santiago.end_of(date));
    }

    #[test]
    fn skipped_times_move_forward_and_repeated_times_take_the_first_pass() {
        let berlin = BotTimezone::new(Tz::Europe__Berlin);

        assert_eq!(
            berlin.parse_deadline("29.03.2026 02:30"),
            Some(at("2026-03-29T01:30:00Z"))
        );
        assert_eq!(
            berlin.parse_deadline("25.10.2026 02:30"),
            Some(at("2026-10-25T00:30:00Z"))
        );
    }

    #[test]
    fn the_label_names_the_city_and_the_offset() {
        assert_eq!(
            vladivostok().label_at(at("2026-08-20T00:00:00Z")),
            "Vladivostok (UTC+10)"
        );
        assert_eq!(
            BotTimezone::new(Tz::Asia__Kolkata).label_at(at("2026-08-20T00:00:00Z")),
            "Kolkata (UTC+5:30)"
        );
        assert_eq!(
            BotTimezone::new(Tz::America__New_York).label_at(at("2026-01-20T00:00:00Z")),
            "New York (UTC-5)"
        );
    }

    #[test]
    fn the_label_follows_daylight_saving_time() {
        let berlin = BotTimezone::new(Tz::Europe__Berlin);

        assert_eq!(
            berlin.label_at(at("2026-01-20T00:00:00Z")),
            "Berlin (UTC+1)"
        );
        assert_eq!(
            berlin.label_at(at("2026-07-20T00:00:00Z")),
            "Berlin (UTC+2)"
        );
    }
}
