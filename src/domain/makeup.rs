use chrono::{DateTime, Duration, Utc};

use crate::domain::{DomainError, DomainResult, Grading, MaxScore, SubmissionKind};

pub const MAX_MAKEUP_ATTEMPTS: u16 = 100;
pub const MAX_MAKEUP_DAYS: i64 = 366;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CourseMakeup {
    until: DateTime<Utc>,
    attempts: u16,
    opened_at: DateTime<Utc>,
}

impl CourseMakeup {
    pub fn open(until: DateTime<Utc>, attempts: u16, now: DateTime<Utc>) -> DomainResult<Self> {
        Self::ensure_until(until, now)?;
        Self::ensure_attempts(attempts)?;

        Ok(Self {
            until,
            attempts,
            opened_at: now,
        })
    }

    pub fn change(
        self,
        until: DateTime<Utc>,
        attempts: u16,
        now: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Self::ensure_until(until, now)?;
        Self::ensure_attempts(attempts)?;

        Ok(Self {
            until,
            attempts,
            ..self
        })
    }

    pub(crate) fn restore(
        until: DateTime<Utc>,
        attempts: u16,
        opened_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Self::ensure_attempts(attempts)?;
        if until <= opened_at {
            return Err(DomainError::InvalidCourseMakeupState);
        }

        Ok(Self {
            until,
            attempts,
            opened_at,
        })
    }

    pub fn ensure_until(until: DateTime<Utc>, now: DateTime<Utc>) -> DomainResult<()> {
        let latest = now.checked_add_signed(Duration::days(MAX_MAKEUP_DAYS));
        if until <= now || latest.is_none_or(|latest| until > latest) {
            return Err(DomainError::MakeupUntilOutOfRange {
                max_days: MAX_MAKEUP_DAYS,
            });
        }

        Ok(())
    }

    pub fn ensure_attempts(attempts: u16) -> DomainResult<()> {
        if attempts == 0 || attempts > MAX_MAKEUP_ATTEMPTS {
            return Err(DomainError::InvalidMakeupAttempts {
                value: attempts,
                max: MAX_MAKEUP_ATTEMPTS,
            });
        }

        Ok(())
    }

    pub fn until(self) -> DateTime<Utc> {
        self.until
    }

    pub fn attempts(self) -> u16 {
        self.attempts
    }

    pub fn opened_at(self) -> DateTime<Utc> {
        self.opened_at
    }

    pub fn is_active_at(self, now: DateTime<Utc>) -> bool {
        now < self.until
    }

    pub fn balance(self, extra: u16, used: u16) -> MakeupBalance {
        MakeupBalance {
            allowed: self.attempts + extra,
            used,
        }
    }

    pub fn grant_attempt(self, extra: u16, now: DateTime<Utc>) -> DomainResult<u16> {
        if !self.is_active_at(now) {
            return Err(DomainError::MakeupNotOffered);
        }
        extra
            .checked_add(1)
            .filter(|extra| *extra <= MAX_MAKEUP_ATTEMPTS)
            .ok_or(DomainError::MakeupGrantLimitExceeded {
                max: MAX_MAKEUP_ATTEMPTS,
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MakeupCredit(u16);

impl MakeupCredit {
    pub fn new(value: u16, max_score: MaxScore) -> DomainResult<Self> {
        if value == 0 || value > max_score.get() {
            return Err(DomainError::InvalidMakeupCredit {
                value,
                max: max_score.get(),
            });
        }

        Ok(Self(value))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MakeupTerms {
    NotOffered,
    Credit(MakeupCredit),
    PassFail,
}

impl MakeupTerms {
    pub fn scored(value: u16, grading: Grading) -> DomainResult<Self> {
        match grading {
            Grading::Score { max_score } => Ok(Self::Credit(MakeupCredit::new(value, max_score)?)),
            Grading::PassFail => Err(DomainError::MakeupTermsDoNotFitGrading),
        }
    }

    pub(crate) fn restore(
        offered: Option<bool>,
        credit: Option<u16>,
        grading: Grading,
    ) -> DomainResult<Option<Self>> {
        let terms = match (offered, credit, grading) {
            (None, None, _) => return Ok(None),
            (Some(false), None, _) => Self::NotOffered,
            (Some(true), None, Grading::PassFail) => Self::PassFail,
            (Some(true), Some(credit), _) => Self::scored(credit, grading)?,
            (Some(true), None, Grading::Score { .. }) | (None | Some(false), Some(_), _) => {
                return Err(DomainError::MakeupTermsDoNotFitGrading);
            }
        };

        Ok(Some(terms))
    }

    pub fn ensure_fits(self, grading: Grading) -> DomainResult<()> {
        let fits = match (self, grading) {
            (Self::NotOffered, _) | (Self::PassFail, Grading::PassFail) => true,
            (Self::Credit(credit), Grading::Score { max_score }) => credit.get() <= max_score.get(),
            (Self::Credit(_), Grading::PassFail) | (Self::PassFail, Grading::Score { .. }) => false,
        };
        if !fits {
            return Err(DomainError::MakeupTermsDoNotFitGrading);
        }

        Ok(())
    }

    pub fn is_offered(self) -> bool {
        self != Self::NotOffered
    }

    pub fn credit(self) -> Option<MakeupCredit> {
        match self {
            Self::Credit(credit) => Some(credit),
            Self::NotOffered | Self::PassFail => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MakeupBalance {
    allowed: u16,
    used: u16,
}

impl MakeupBalance {
    pub fn allowed(self) -> u16 {
        self.allowed
    }

    pub fn used(self) -> u16 {
        self.used
    }

    pub fn remaining(self) -> u16 {
        self.allowed.saturating_sub(self.used)
    }

    pub fn spend(self) -> Self {
        Self {
            used: self.used + 1,
            ..self
        }
    }

    pub(crate) fn ensure_available(self) -> DomainResult<()> {
        if self.remaining() == 0 {
            return Err(DomainError::MakeupAttemptsExhausted {
                allowed: self.allowed,
                used: self.used,
            });
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MakeupRequest {
    pub course: Option<CourseMakeup>,
    pub consent: bool,
    pub balance: MakeupBalance,
}

impl MakeupRequest {
    pub fn consented(course: Option<CourseMakeup>, balance: MakeupBalance) -> Self {
        Self {
            course,
            consent: true,
            balance,
        }
    }

    pub(crate) fn consented_course(self) -> Option<CourseMakeup> {
        self.course.filter(|_| self.consent)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acceptance {
    Regular,
    Makeup { credit: Option<MakeupCredit> },
}

impl Acceptance {
    pub fn kind(self) -> SubmissionKind {
        match self {
            Self::Regular => SubmissionKind::Regular,
            Self::Makeup { .. } => SubmissionKind::Makeup,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Offer {
    Regular,
    Makeup {
        credit: Option<MakeupCredit>,
        until: DateTime<Utc>,
        balance: MakeupBalance,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::t0;

    fn max(value: u16) -> MaxScore {
        MaxScore::new(value).unwrap()
    }

    #[test]
    fn the_end_of_makeup_lies_after_now_and_at_most_366_days_ahead() {
        let now = t0();
        let latest = now + Duration::days(MAX_MAKEUP_DAYS);
        let out_of_range = DomainError::MakeupUntilOutOfRange {
            max_days: MAX_MAKEUP_DAYS,
        };

        for until in [
            now - Duration::days(1),
            now,
            latest + Duration::nanoseconds(1),
        ] {
            assert_eq!(CourseMakeup::open(until, 3, now), Err(out_of_range.clone()));
        }
        for until in [now + Duration::nanoseconds(1), latest] {
            let makeup = CourseMakeup::open(until, 3, now).unwrap();
            assert_eq!(makeup.until(), until);
            assert_eq!(makeup.opened_at(), now);
        }
    }

    #[test]
    fn the_pool_size_lies_between_one_and_a_hundred() {
        let until = t0() + Duration::days(30);

        for attempts in [0, MAX_MAKEUP_ATTEMPTS + 1] {
            assert_eq!(
                CourseMakeup::open(until, attempts, t0()),
                Err(DomainError::InvalidMakeupAttempts {
                    value: attempts,
                    max: MAX_MAKEUP_ATTEMPTS,
                })
            );
        }
        for attempts in [1, MAX_MAKEUP_ATTEMPTS] {
            assert_eq!(
                CourseMakeup::open(until, attempts, t0())
                    .unwrap()
                    .attempts(),
                attempts
            );
        }
    }

    #[test]
    fn a_change_keeps_the_opening_and_checks_the_end_against_now() {
        let opened = CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap();
        let later = t0() + Duration::days(10);

        let changed = opened.change(opened.until(), 4, later).unwrap();
        assert_eq!(changed.attempts(), 4);
        assert_eq!(changed.opened_at(), t0());
        assert_eq!(
            opened.change(later - Duration::days(1), 4, later),
            Err(DomainError::MakeupUntilOutOfRange {
                max_days: MAX_MAKEUP_DAYS,
            })
        );
        assert_eq!(
            opened
                .change(later + Duration::days(MAX_MAKEUP_DAYS), 3, later)
                .unwrap()
                .opened_at(),
            t0()
        );
    }

    #[test]
    fn makeup_is_active_strictly_before_its_end() {
        let makeup = CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap();

        assert!(makeup.is_active_at(t0()));
        assert!(makeup.is_active_at(makeup.until() - Duration::nanoseconds(1)));
        assert!(!makeup.is_active_at(makeup.until()));
    }

    #[test]
    fn stored_settings_end_after_their_opening() {
        assert_eq!(
            CourseMakeup::restore(t0(), 3, t0()),
            Err(DomainError::InvalidCourseMakeupState)
        );
        assert_eq!(
            CourseMakeup::restore(t0() - Duration::days(1), 3, t0() - Duration::days(30))
                .unwrap()
                .until(),
            t0() - Duration::days(1)
        );
    }

    #[test]
    fn a_shrunk_pool_leaves_nothing_instead_of_a_negative_balance() {
        let makeup = CourseMakeup::open(t0() + Duration::days(30), 1, t0()).unwrap();
        let balance = makeup.balance(0, 2);

        assert_eq!(balance.remaining(), 0);
        assert_eq!(
            balance.ensure_available(),
            Err(DomainError::MakeupAttemptsExhausted {
                allowed: 1,
                used: 2,
            })
        );
        assert_eq!(makeup.balance(2, 2).remaining(), 1);
    }

    #[test]
    fn a_hundred_attempts_can_be_granted_per_opening() {
        let makeup = CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap();

        assert_eq!(makeup.grant_attempt(0, t0()), Ok(1));
        assert_eq!(
            makeup.grant_attempt(MAX_MAKEUP_ATTEMPTS - 1, t0()),
            Ok(MAX_MAKEUP_ATTEMPTS)
        );
        assert_eq!(
            makeup.grant_attempt(MAX_MAKEUP_ATTEMPTS, t0()),
            Err(DomainError::MakeupGrantLimitExceeded {
                max: MAX_MAKEUP_ATTEMPTS,
            })
        );
    }

    #[test]
    fn no_attempt_is_granted_after_the_makeup_ended() {
        let makeup = CourseMakeup::open(t0() + Duration::days(30), 3, t0()).unwrap();

        assert_eq!(
            makeup.grant_attempt(0, makeup.until()),
            Err(DomainError::MakeupNotOffered)
        );
    }

    #[test]
    fn a_credit_fits_only_its_own_kind_of_scale_within_the_maximum() {
        let ten = Grading::scored(10).unwrap();
        let five = Grading::scored(5).unwrap();
        let six = MakeupTerms::scored(6, ten).unwrap();

        assert_eq!(six.ensure_fits(ten), Ok(()));
        assert_eq!(
            six.ensure_fits(five),
            Err(DomainError::MakeupTermsDoNotFitGrading)
        );
        assert_eq!(
            six.ensure_fits(Grading::PassFail),
            Err(DomainError::MakeupTermsDoNotFitGrading)
        );
        assert_eq!(
            MakeupTerms::PassFail.ensure_fits(ten),
            Err(DomainError::MakeupTermsDoNotFitGrading)
        );
        for grading in [ten, five, Grading::PassFail] {
            assert_eq!(MakeupTerms::NotOffered.ensure_fits(grading), Ok(()));
        }
    }

    #[test]
    fn a_credit_lies_between_one_and_the_maximum_of_the_scale() {
        for value in [0, 11] {
            assert_eq!(
                MakeupCredit::new(value, max(10)),
                Err(DomainError::InvalidMakeupCredit { value, max: 10 })
            );
        }
        assert_eq!(MakeupCredit::new(10, max(10)).unwrap().get(), 10);
        assert_eq!(
            MakeupTerms::scored(1, Grading::PassFail),
            Err(DomainError::MakeupTermsDoNotFitGrading)
        );
    }

    #[test]
    fn stored_terms_restore_every_state_and_reject_a_number_without_an_offer() {
        let ten = Grading::scored(10).unwrap();

        assert_eq!(MakeupTerms::restore(None, None, ten), Ok(None));
        assert_eq!(
            MakeupTerms::restore(Some(false), None, ten),
            Ok(Some(MakeupTerms::NotOffered))
        );
        assert_eq!(
            MakeupTerms::restore(Some(true), None, Grading::PassFail),
            Ok(Some(MakeupTerms::PassFail))
        );
        assert_eq!(
            MakeupTerms::restore(Some(true), Some(6), ten)
                .unwrap()
                .and_then(MakeupTerms::credit)
                .map(MakeupCredit::get),
            Some(6)
        );
        for (offered, credit, grading) in [
            (None, Some(6), ten),
            (Some(false), Some(6), ten),
            (Some(true), None, ten),
            (Some(true), Some(6), Grading::PassFail),
        ] {
            assert_eq!(
                MakeupTerms::restore(offered, credit, grading),
                Err(DomainError::MakeupTermsDoNotFitGrading),
                "{offered:?} {credit:?} {grading:?}"
            );
        }
    }
}
