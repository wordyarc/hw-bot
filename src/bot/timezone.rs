use chrono::{DateTime, Days, FixedOffset, Months, NaiveDate, NaiveTime, TimeZone, Utc};

#[derive(Clone, Copy, Debug)]
pub struct BotTimezone {
    offset: FixedOffset,
    deadline_time: NaiveTime,
}

impl BotTimezone {
    pub fn new(offset: FixedOffset, deadline_time: NaiveTime) -> Self {
        Self {
            offset,
            deadline_time,
        }
    }

    pub fn deadline_time(&self) -> NaiveTime {
        self.deadline_time
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
        Utc::now().with_timezone(&self.offset).date_naive()
    }

    pub fn format(&self, instant: DateTime<Utc>) -> String {
        format!(
            "{} ({})",
            instant.with_timezone(&self.offset).format("%d.%m.%Y %H:%M"),
            self.suffix(),
        )
    }

    pub fn format_short(&self, instant: DateTime<Utc>) -> String {
        instant
            .with_timezone(&self.offset)
            .format("%d.%m %H:%M")
            .to_string()
    }

    pub fn format_date(&self, date: NaiveDate) -> String {
        date.format("%d.%m.%Y").to_string()
    }

    pub fn end_of(&self, date: NaiveDate) -> Option<DateTime<Utc>> {
        self.instant(date, self.deadline_time)
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
            let date = NaiveDate::parse_from_str(date.trim(), "%d.%m.%Y").ok()?;
            let time = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
            self.instant(date, time)
        } else {
            self.end_of(NaiveDate::parse_from_str(text, "%d.%m.%Y").ok()?)
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
        BotTimezone::new(
            FixedOffset::east_opt(10 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        )
    }

    #[test]
    fn deadlines_are_read_and_shown_in_the_configured_timezone() {
        let timezone = vladivostok();
        let deadline = timezone.parse_deadline("20.08.2026 23:59").unwrap();

        assert_eq!(
            deadline,
            "2026-08-20T13:59:00Z".parse::<DateTime<Utc>>().unwrap()
        );
        assert_eq!(timezone.format(deadline), "20.08.2026 23:59 (UTC+10)");
    }

    #[test]
    fn a_date_without_time_expires_at_the_configured_end_of_day() {
        let timezone = vladivostok();

        assert_eq!(
            timezone.parse_deadline("20.08.2026"),
            timezone.parse_deadline("20.08.2026 23:59")
        );
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
        let timezone = BotTimezone::new(
            FixedOffset::east_opt(5 * 3600 + 1800).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        assert_eq!(timezone.suffix(), "UTC+5:30");
    }

    #[test]
    fn western_offsets_keep_their_sign() {
        let timezone = BotTimezone::new(
            FixedOffset::west_opt(5 * 3600).unwrap(),
            NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
        );

        assert_eq!(timezone.suffix(), "UTC-5");
    }
}
