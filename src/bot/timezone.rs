use chrono::{DateTime, Days, FixedOffset, Months, NaiveDate, NaiveTime, TimeZone, Utc};

/// Границы сроков исключительные, как у интервалов RFC 5545: «до конца дня 21.09» хранится как
/// 22.09 00:00 в поясе установки, и в этот момент приём уже закрыт.
#[derive(Clone, Copy, Debug)]
pub struct BotTimezone {
    offset: FixedOffset,
}

const DATE: &str = "%d.%m.%Y";
const DATE_TIME: &str = "%d.%m.%Y %H:%M";
const SHORT_DATE: &str = "%d.%m";
const SHORT_DATE_TIME: &str = "%d.%m %H:%M";

impl BotTimezone {
    pub fn new(offset: FixedOffset) -> Self {
        Self { offset }
    }

    /// `UTC+10`
    pub fn suffix(&self) -> String {
        let total_minutes = self.offset.local_minus_utc() / 60;
        let sign = if total_minutes < 0 { '-' } else { '+' };
        let hours = total_minutes.abs() / 60;
        let minutes = total_minutes.abs() % 60;

        if minutes == 0 {
            format!("UTC{sign}{hours}")
        } else {
            format!("UTC{sign}{hours}:{minutes:02}")
        }
    }

    pub fn today(&self) -> NaiveDate {
        self.local_date(Utc::now())
    }

    pub fn local_date(&self, instant: DateTime<Utc>) -> NaiveDate {
        instant.with_timezone(&self.offset).date_naive()
    }

    pub fn first_date_ending_after(&self, instant: DateTime<Utc>) -> NaiveDate {
        self.local_date(instant)
    }

    pub fn format(&self, instant: DateTime<Utc>) -> String {
        format!("до {} ({})", self.format_until(instant), self.suffix())
    }

    pub fn format_closed_since(&self, instant: DateTime<Utc>) -> String {
        format!("с {} ({})", self.format_since(instant), self.suffix())
    }

    pub fn format_until(&self, instant: DateTime<Utc>) -> String {
        self.end_label(instant, DATE, DATE_TIME)
    }

    pub fn format_until_short(&self, instant: DateTime<Utc>) -> String {
        self.end_label(instant, SHORT_DATE, SHORT_DATE_TIME)
    }

    pub fn format_since(&self, instant: DateTime<Utc>) -> String {
        let local = instant.with_timezone(&self.offset);
        if local.time() == NaiveTime::MIN {
            local.format(DATE).to_string()
        } else {
            local.format(DATE_TIME).to_string()
        }
    }

    fn end_label(&self, instant: DateTime<Utc>, date: &str, date_time: &str) -> String {
        let local = instant.with_timezone(&self.offset);
        match local.date_naive().pred_opt() {
            Some(day) if local.time() == NaiveTime::MIN => {
                format!("конца дня {}", day.format(date))
            }
            _ => local.format(date_time).to_string(),
        }
    }

    pub fn format_short(&self, instant: DateTime<Utc>) -> String {
        instant
            .with_timezone(&self.offset)
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
        self.offset
            .from_local_datetime(&date.and_time(time))
            .single()
            .map(|instant| instant.with_timezone(&Utc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vladivostok() -> BotTimezone {
        BotTimezone::new(FixedOffset::east_opt(10 * 3600).unwrap())
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

        assert_eq!(
            timezone.format(deadline),
            "до конца дня 20.08.2026 (UTC+10)"
        );
        assert_eq!(timezone.format_until(deadline), "конца дня 20.08.2026");
        assert_eq!(timezone.format_until_short(deadline), "конца дня 20.08");
        assert_eq!(timezone.format_since(deadline), "21.08.2026");
        assert_eq!(
            timezone.format_closed_since(deadline),
            "с 21.08.2026 (UTC+10)"
        );
    }

    #[test]
    fn explicit_times_are_read_and_shown_in_the_configured_timezone() {
        let timezone = vladivostok();
        let deadline = timezone.parse_deadline("20.08.2026 15:00").unwrap();

        assert_eq!(
            deadline,
            "2026-08-20T05:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(timezone.format(deadline), "до 20.08.2026 15:00 (UTC+10)");
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
    fn half_hour_offsets_keep_their_minutes_in_the_suffix() {
        let timezone = BotTimezone::new(FixedOffset::east_opt(5 * 3600 + 1800).unwrap());

        assert_eq!(timezone.suffix(), "UTC+5:30");
    }

    #[test]
    fn western_offsets_keep_their_sign() {
        let timezone = BotTimezone::new(FixedOffset::west_opt(5 * 3600).unwrap());

        assert_eq!(timezone.suffix(), "UTC-5");
    }
}
