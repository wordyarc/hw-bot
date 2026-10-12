use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Duration, Utc};

use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, Acceptance, AssignmentId, CourseMakeup, DomainError, DomainResult,
    Grade, MakeupTerms, NonEmptyString, ReviewScore, StudentGroupId, SubjectId, TeachingCourse,
    TeachingCourseId, USER_TEXT_MAX_LEN, User, UserId,
};
use crate::string_enum::str_enum;

pub const MAX_ATTEMPTS: u16 = i16::MAX as u16;
pub const MAX_SCORE: u16 = 100;
pub const MAX_LATE_PERIOD_DAYS: u16 = 365;
pub const MAX_PENALTY_PER_PERIOD: u16 = MAX_SCORE;
pub const MAX_LATE_PERIODS: u16 = 100;
pub const MAX_LATENESS_DAYS: u32 = 366;
/// Год выбран по формату ДД.ММ.ГГГГ. Граница нужна не только для показа: у края диапазона chrono
/// перевод момента в пояс установки паникует, а сроки печатаются в каждом экране задания.
pub const LATEST_TERM_YEAR: i32 = 9999;

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AssignmentStatus {
        Draft => "draft",
        Published => "published",
        Archived => "archived",
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum DeductedFrom {
        WorkScore => "work_score",
        CourseTotal => "course_total",
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Grading {
    #[default]
    PassFail,
    Score {
        max_score: MaxScore,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaxScore(u16);

impl MaxScore {
    pub fn new(max_score: u16) -> DomainResult<Self> {
        if max_score == 0 || max_score > MAX_SCORE {
            return Err(DomainError::InvalidMaxScore {
                value: max_score,
                max: MAX_SCORE,
            });
        }

        Ok(Self(max_score))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl Grading {
    pub fn scored(max_score: u16) -> DomainResult<Self> {
        Ok(Self::Score {
            max_score: MaxScore::new(max_score)?,
        })
    }

    pub fn from_max_score(max_score: Option<u16>) -> DomainResult<Self> {
        max_score.map_or(Ok(Self::PassFail), Self::scored)
    }

    pub fn max_score(self) -> Option<u16> {
        match self {
            Self::PassFail => None,
            Self::Score { max_score } => Some(max_score.get()),
        }
    }

    pub fn is_scored(self) -> bool {
        matches!(self, Self::Score { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    id: AssignmentId,
    teaching_course_id: TeachingCourseId,
    subject_id: SubjectId,
    groups: BTreeSet<StudentGroupId>,
    title: NonEmptyString,
    body: NonEmptyString,
    max_attempts: u16,
    grading: Grading,
    late_policy: Option<LatePolicy>,
    makeup: Option<MakeupTerms>,
    status: AssignmentStatus,
    created_by: UserId,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatePolicy {
    period_days: u16,
    penalty_per_period: u16,
    max_periods: u16,
    deducted_from: DeductedFrom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatePenalty {
    periods: u32,
    points: u16,
    deducted_from: DeductedFrom,
}

impl LatePenalty {
    pub(crate) fn restore(periods: u32, points: u16, deducted_from: DeductedFrom) -> Option<Self> {
        (periods > 0 && (1..=MAX_SCORE).contains(&points)).then_some(Self {
            periods,
            points,
            deducted_from,
        })
    }

    pub fn periods(self) -> u32 {
        self.periods
    }

    pub fn points(self) -> u16 {
        self.points
    }

    pub fn deducted_from(self) -> DeductedFrom {
        self.deducted_from
    }
}

impl LatePolicy {
    pub fn new(
        period_days: u16,
        penalty_per_period: u16,
        max_periods: u16,
        deducted_from: DeductedFrom,
    ) -> DomainResult<Self> {
        check_range("period_days", period_days, MAX_LATE_PERIOD_DAYS)?;
        check_range(
            "penalty_per_period",
            penalty_per_period,
            MAX_PENALTY_PER_PERIOD,
        )?;
        check_range("max_periods", max_periods, MAX_LATE_PERIODS)?;

        Ok(Self {
            period_days,
            penalty_per_period,
            max_periods,
            deducted_from,
        })
    }

    pub fn period_days(self) -> u16 {
        self.period_days
    }

    pub fn penalty_per_period(self) -> u16 {
        self.penalty_per_period
    }

    pub fn max_periods(self) -> u16 {
        self.max_periods
    }

    pub fn deducted_from(self) -> DeductedFrom {
        self.deducted_from
    }

    pub fn period_end(
        self,
        deadline_at: DateTime<Utc>,
        periods: u16,
    ) -> DomainResult<DateTime<Utc>> {
        let days = u32::from(self.period_days) * u32::from(periods);
        deadline_at
            .checked_add_signed(Duration::days(i64::from(days)))
            .ok_or(DomainError::AssignmentClosingOutOfRange)
            .and_then(supported_term)
    }

    pub fn closes_at(self, deadline_at: DateTime<Utc>) -> DomainResult<DateTime<Utc>> {
        self.period_end(deadline_at, self.max_periods)
    }

    pub fn validate_horizon(self, deadline_at: DateTime<Utc>) -> DomainResult<()> {
        self.closes_at(deadline_at)?;
        if u32::from(self.period_days) * u32::from(self.max_periods) > MAX_LATENESS_DAYS {
            return Err(DomainError::LateHorizonTooLong {
                max_days: MAX_LATENESS_DAYS,
            });
        }
        Ok(())
    }

    pub fn possible_score(self, max_score: MaxScore, periods: u32) -> u16 {
        Grade::new(max_score.get(), max_score.get(), self.late_penalty(periods)).work_score()
    }

    pub fn late_penalty(self, periods: u32) -> Option<LatePenalty> {
        (periods > 0).then(|| LatePenalty {
            periods,
            points: self.penalty_for(periods),
            deducted_from: self.deducted_from,
        })
    }

    pub fn periods_until(self, deadline_at: DateTime<Utc>, at: DateTime<Utc>) -> u32 {
        if at < deadline_at {
            return 0;
        }
        self.whole_periods(deadline_at, at).0 + 1
    }

    pub fn periods_to_close_at(self, deadline_at: DateTime<Utc>, closes_at: DateTime<Utc>) -> u32 {
        if closes_at <= deadline_at {
            return 0;
        }
        let (whole, rest) = self.whole_periods(deadline_at, closes_at);
        whole + u32::from(rest)
    }

    fn whole_periods(self, deadline_at: DateTime<Utc>, at: DateTime<Utc>) -> (u32, bool) {
        let lateness = at - deadline_at;
        let period = Duration::days(i64::from(self.period_days)).num_seconds();
        let whole = lateness.num_seconds() / period;
        let rest = lateness > Duration::seconds(whole * period);

        (
            u32::try_from(whole).expect("chrono's date range fits in u32 days"),
            rest,
        )
    }

    pub fn penalty_for(self, late_periods: u32) -> u16 {
        late_periods
            .saturating_mul(u32::from(self.penalty_per_period))
            .min(u32::from(MAX_SCORE)) as u16
    }
}

fn supported_term(at: DateTime<Utc>) -> DomainResult<DateTime<Utc>> {
    if at.year() > LATEST_TERM_YEAR {
        return Err(DomainError::AssignmentClosingOutOfRange);
    }
    Ok(at)
}

fn check_range(field: &'static str, value: u16, max: u16) -> DomainResult<()> {
    if value == 0 || value > max {
        return Err(DomainError::InvalidLatePolicy { field, value, max });
    }

    Ok(())
}

pub(crate) struct AssignmentParts {
    pub id: AssignmentId,
    pub teaching_course_id: TeachingCourseId,
    pub subject_id: SubjectId,
    pub groups: BTreeSet<StudentGroupId>,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub makeup: Option<MakeupTerms>,
    pub status: AssignmentStatus,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy)]
struct ProposedTerms {
    grading: Grading,
    late_policy: Option<LatePolicy>,
    deadline_at: DateTime<Utc>,
    makeup: Option<MakeupTerms>,
}

#[derive(Clone, Debug)]
pub struct NewAssignment {
    pub title: String,
    pub body: String,
    pub groups: Option<BTreeSet<StudentGroupId>>,
    pub max_attempts: u16,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub makeup: Option<MakeupTerms>,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
}

impl Assignment {
    pub(crate) fn restore(parts: AssignmentParts) -> DomainResult<Self> {
        if parts.max_attempts == 0 || parts.max_attempts > MAX_ATTEMPTS {
            return Err(DomainError::InvalidAttemptLimit {
                value: parts.max_attempts,
            });
        }

        Self::validate_terms(
            parts.created_at,
            ProposedTerms {
                grading: parts.grading,
                late_policy: parts.late_policy,
                deadline_at: parts.deadline_at,
                makeup: parts.makeup,
            },
        )?;

        // У архивного задания может не быть published_at, если его убрали туда из черновика.
        match parts.status {
            AssignmentStatus::Draft if parts.published_at.is_some() => {
                return Err(DomainError::InvalidAssignmentPublishedState);
            }
            AssignmentStatus::Published if parts.published_at.is_none() => {
                return Err(DomainError::InvalidAssignmentPublishedState);
            }
            _ => {}
        }

        Ok(Self {
            id: parts.id,
            teaching_course_id: parts.teaching_course_id,
            subject_id: parts.subject_id,
            groups: parts.groups,
            title: NonEmptyString::new_bounded(
                "assignment.title",
                parts.title,
                ASSIGNMENT_TITLE_MAX_LEN,
            )?,
            body: NonEmptyString::new_bounded("assignment.body", parts.body, USER_TEXT_MAX_LEN)?,
            max_attempts: parts.max_attempts,
            grading: parts.grading,
            late_policy: parts.late_policy,
            makeup: parts.makeup,
            status: parts.status,
            created_by: parts.created_by,
            created_at: parts.created_at,
            deadline_at: parts.deadline_at,
            published_at: parts.published_at,
        })
    }

    pub fn new(
        id: AssignmentId,
        teaching_course: &TeachingCourse,
        created_by: &User,
        input: NewAssignment,
    ) -> DomainResult<Self> {
        if !teaching_course.can_be_managed_by(created_by) {
            return Err(DomainError::PermissionDenied {
                action: "create assignment",
            });
        }

        if let Some(policy) = input.late_policy {
            policy.validate_horizon(input.deadline_at)?;
        }

        let groups = match input.groups {
            None => teaching_course.group_ids().clone(),
            Some(groups) if groups.is_empty() => return Err(DomainError::AssignmentHasNoGroups),
            Some(groups) if !groups.is_subset(teaching_course.group_ids()) => {
                return Err(DomainError::AssignmentGroupOutsideCourse);
            }
            Some(groups) => groups,
        };

        Self::restore(AssignmentParts {
            id,
            teaching_course_id: teaching_course.id(),
            subject_id: teaching_course.subject_id(),
            groups,
            title: input.title,
            body: input.body,
            max_attempts: input.max_attempts,
            grading: input.grading,
            late_policy: input.late_policy,
            makeup: input.makeup,
            status: AssignmentStatus::Draft,
            created_by: created_by.id(),
            created_at: input.created_at,
            deadline_at: input.deadline_at,
            published_at: None,
        })
    }

    pub fn publish(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        published_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "publish assignment")?;

        if self.status == AssignmentStatus::Published {
            return Err(DomainError::AssignmentStatusTransitionNotAllowed {
                from: self.status,
                to: AssignmentStatus::Published,
            });
        }

        self.status = AssignmentStatus::Published;
        self.published_at = Some(published_at);

        Ok(())
    }

    pub fn archive(&mut self, teaching_course: &TeachingCourse, actor: &User) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "archive assignment")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentStatusTransitionNotAllowed {
                from: self.status,
                to: AssignmentStatus::Archived,
            });
        }

        self.status = AssignmentStatus::Archived;

        Ok(())
    }

    pub fn change_deadline(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        new_deadline_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.change_terms(
            teaching_course,
            actor,
            new_deadline_at,
            self.late_policy,
            self.makeup,
        )
    }

    pub fn change_terms(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        new_deadline_at: DateTime<Utc>,
        late_policy: Option<LatePolicy>,
        makeup: Option<MakeupTerms>,
    ) -> DomainResult<()> {
        self.ensure_terms_change(
            teaching_course,
            actor,
            "change assignment deadline",
            ProposedTerms {
                grading: self.grading,
                late_policy,
                deadline_at: new_deadline_at,
                makeup,
            },
        )?;
        self.late_policy = late_policy;
        self.deadline_at = new_deadline_at;
        self.makeup = makeup;

        Ok(())
    }

    pub fn change_grading(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        grading: Grading,
        late_policy: Option<LatePolicy>,
    ) -> DomainResult<()> {
        self.ensure_terms_change(
            teaching_course,
            actor,
            "change assignment grading",
            ProposedTerms {
                grading,
                late_policy,
                deadline_at: self.deadline_at,
                makeup: self.makeup,
            },
        )?;
        self.grading = grading;
        self.late_policy = late_policy;

        Ok(())
    }

    pub fn decide_makeup(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        terms: MakeupTerms,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "decide assignment makeup")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentIsArchived);
        }
        terms.ensure_fits(self.grading)?;
        self.makeup = Some(terms);

        Ok(())
    }

    fn ensure_terms_change(
        &self,
        teaching_course: &TeachingCourse,
        actor: &User,
        action: &'static str,
        terms: ProposedTerms,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, action)?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentIsArchived);
        }

        Self::validate_terms(self.created_at, terms)?;
        if let Some(policy) = terms.late_policy {
            policy.validate_horizon(terms.deadline_at)?;
        }

        Ok(())
    }

    fn validate_terms(created_at: DateTime<Utc>, terms: ProposedTerms) -> DomainResult<()> {
        if terms.deadline_at <= created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }
        if terms.late_policy.is_some() && !terms.grading.is_scored() {
            return Err(DomainError::LatePolicyRequiresScoredGrading);
        }
        Self::closing_for(terms.deadline_at, terms.late_policy)?;
        if let Some(makeup) = terms.makeup {
            makeup.ensure_fits(terms.grading)?;
        }

        Ok(())
    }

    pub fn id(&self) -> AssignmentId {
        self.id
    }

    pub fn teaching_course_id(&self) -> TeachingCourseId {
        self.teaching_course_id
    }

    pub fn subject_id(&self) -> SubjectId {
        self.subject_id
    }

    pub fn groups(&self) -> &BTreeSet<StudentGroupId> {
        &self.groups
    }

    pub fn is_issued_to(&self, student: &User) -> bool {
        student
            .student_group_id()
            .is_some_and(|group_id| self.groups.contains(&group_id))
    }

    pub fn ensure_belongs_to_course(&self, teaching_course: &TeachingCourse) -> DomainResult<()> {
        if self.teaching_course_id != teaching_course.id() {
            return Err(DomainError::AssignmentDoesNotBelongToTeachingCourse);
        }

        Ok(())
    }

    pub(crate) fn ensure_managed_by(
        &self,
        teaching_course: &TeachingCourse,
        actor: &User,
        action: &'static str,
    ) -> DomainResult<()> {
        self.ensure_belongs_to_course(teaching_course)?;

        if !teaching_course.can_be_managed_by(actor) {
            return Err(DomainError::PermissionDenied { action });
        }

        Ok(())
    }

    pub fn title(&self) -> &NonEmptyString {
        &self.title
    }

    pub fn body(&self) -> &NonEmptyString {
        &self.body
    }

    pub fn max_attempts(&self) -> u16 {
        self.max_attempts
    }

    pub fn grading(&self) -> Grading {
        self.grading
    }

    pub fn late_policy(&self) -> Option<LatePolicy> {
        self.late_policy
    }

    pub fn makeup(&self) -> Option<MakeupTerms> {
        self.makeup
    }

    pub(crate) fn regular_grade(
        &self,
        score: Option<ReviewScore>,
        submitted_at: DateTime<Utc>,
    ) -> DomainResult<Option<Grade>> {
        match (self.grading, score) {
            (Grading::PassFail, None) => Ok(None),
            (Grading::PassFail, Some(_)) => Err(DomainError::ScoreNotAllowed),
            (Grading::Score { .. }, None) => Err(DomainError::ScoreRequired),
            (Grading::Score { max_score }, Some(score)) if score.max_score != max_score.get() => {
                Err(DomainError::ScoreScaleMismatch {
                    given_max: score.max_score,
                    max: max_score.get(),
                })
            }
            (Grading::Score { max_score }, Some(score)) if score.value > max_score.get() => {
                Err(DomainError::ScoreOutOfRange {
                    value: score.value,
                    max: max_score.get(),
                })
            }
            (Grading::Score { max_score }, Some(score)) => Ok(Some(Grade::new(
                score.value,
                max_score.get(),
                self.late_penalty_at(submitted_at),
            ))),
        }
    }

    pub fn makeup_grade(&self) -> DomainResult<Option<Grade>> {
        match (self.makeup, self.grading) {
            (Some(MakeupTerms::Credit(credit)), Grading::Score { max_score }) => {
                Ok(Some(Grade::new(credit.get(), max_score.get(), None)))
            }
            (Some(MakeupTerms::PassFail), Grading::PassFail) => Ok(None),
            _ => Err(DomainError::MakeupNotOffered),
        }
    }

    pub fn closes_at(&self) -> DateTime<Utc> {
        Self::closing_for(self.deadline_at, self.late_policy).expect("validated assignment horizon")
    }

    /// Без политики опозданий приём закрывает сам дедлайн.
    pub fn closing_for(
        deadline_at: DateTime<Utc>,
        late_policy: Option<LatePolicy>,
    ) -> DomainResult<DateTime<Utc>> {
        match late_policy {
            Some(policy) => policy.closes_at(deadline_at),
            None => supported_term(deadline_at),
        }
    }

    pub fn late_periods_at(&self, at: DateTime<Utc>) -> u32 {
        self.late_policy
            .map_or(0, |policy| policy.periods_until(self.deadline_at, at))
    }

    pub fn late_penalty_at(&self, at: DateTime<Utc>) -> Option<LatePenalty> {
        self.late_policy
            .and_then(|policy| policy.late_penalty(self.late_periods_at(at)))
    }

    pub fn ensure_open_for_submission(&self, at: DateTime<Utc>) -> DomainResult<()> {
        let closes_at = self.closes_at();
        if at < closes_at {
            return Ok(());
        }

        Err(match self.late_policy {
            None => DomainError::AssignmentDeadlineExceeded {
                deadline_at: self.deadline_at,
                submitted_at: at,
            },
            Some(_) => DomainError::SubmissionWindowClosed {
                closes_at,
                submitted_at: at,
            },
        })
    }

    pub fn acceptance_at(
        &self,
        at: DateTime<Utc>,
        course_makeup: Option<CourseMakeup>,
    ) -> DomainResult<Acceptance> {
        let Err(closed) = self.ensure_open_for_submission(at) else {
            return Ok(Acceptance::Regular);
        };
        let Some(course_makeup) = course_makeup else {
            return Err(closed);
        };
        let credit = match self.makeup {
            Some(MakeupTerms::Credit(credit)) => Some(credit),
            Some(MakeupTerms::PassFail) => None,
            Some(MakeupTerms::NotOffered) | None => return Err(closed),
        };
        let opens_at = self.closes_at().max(course_makeup.opened_at());
        if at < opens_at || at >= course_makeup.until() {
            return Err(closed);
        }

        Ok(Acceptance::Makeup { credit })
    }

    pub fn status(&self) -> AssignmentStatus {
        self.status
    }

    pub fn created_by(&self) -> UserId {
        self.created_by
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub fn deadline_at(&self) -> DateTime<Utc> {
        self.deadline_at
    }

    pub fn published_at(&self) -> Option<DateTime<Utc>> {
        self.published_at
    }

    pub fn is_published(&self) -> bool {
        self.status == AssignmentStatus::Published
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::test_support::{
        assignment, course_for, student, student_group, subject, t0, teacher, teaching_course,
    };

    fn restore(
        status: AssignmentStatus,
        max_attempts: u16,
        published_at: Option<DateTime<Utc>>,
    ) -> DomainResult<Assignment> {
        let created_at = t0();

        Assignment::restore(AssignmentParts {
            id: AssignmentId::new(),
            teaching_course_id: TeachingCourseId::new(),
            subject_id: SubjectId::new(),
            groups: BTreeSet::from([StudentGroupId::new()]),
            title: "Rust ownership".to_owned(),
            body: "Send a pull request link.".to_owned(),
            max_attempts,
            grading: Grading::PassFail,
            late_policy: None,
            makeup: None,
            status,
            created_by: UserId::new(),
            created_at,
            deadline_at: created_at + Duration::days(7),
            published_at,
        })
    }

    #[test]
    fn restores_published_assignment() {
        let published_at = t0();
        let assignment = restore(AssignmentStatus::Published, 2, Some(published_at)).unwrap();

        assert!(assignment.is_published());
        assert_eq!(assignment.published_at(), Some(published_at));
        assert_eq!(assignment.max_attempts(), 2);
    }

    #[test]
    fn restores_archived_assignment_regardless_of_published_at() {
        assert!(restore(AssignmentStatus::Archived, 1, None).is_ok());
        assert!(restore(AssignmentStatus::Archived, 1, Some(t0())).is_ok());
    }

    #[test]
    fn restore_rejects_zero_max_attempts() {
        assert_eq!(
            restore(AssignmentStatus::Draft, 0, None).unwrap_err(),
            DomainError::InvalidAttemptLimit { value: 0 }
        );
    }

    #[test]
    fn restore_rejects_max_attempts_that_do_not_fit_storage() {
        assert_eq!(
            restore(AssignmentStatus::Draft, MAX_ATTEMPTS + 1, None).unwrap_err(),
            DomainError::InvalidAttemptLimit {
                value: MAX_ATTEMPTS + 1,
            }
        );
    }

    #[test]
    fn restore_rejects_published_assignment_without_published_at() {
        assert_eq!(
            restore(AssignmentStatus::Published, 1, None).unwrap_err(),
            DomainError::InvalidAssignmentPublishedState
        );
    }

    #[test]
    fn grading_rejects_max_score_outside_the_scale() {
        for value in [0, MAX_SCORE + 1] {
            assert_eq!(
                Grading::scored(value).unwrap_err(),
                DomainError::InvalidMaxScore {
                    value,
                    max: MAX_SCORE,
                }
            );
        }
    }

    #[test]
    fn late_policy_rejects_each_field_outside_its_range() {
        for (field, max) in [
            ("period_days", MAX_LATE_PERIOD_DAYS),
            ("penalty_per_period", MAX_PENALTY_PER_PERIOD),
            ("max_periods", MAX_LATE_PERIODS),
        ] {
            for value in [0, max + 1] {
                let policy = match field {
                    "period_days" => LatePolicy::new(value, 1, 1, DeductedFrom::WorkScore),
                    "penalty_per_period" => LatePolicy::new(1, value, 1, DeductedFrom::WorkScore),
                    _ => LatePolicy::new(1, 1, value, DeductedFrom::WorkScore),
                };

                assert_eq!(
                    policy.unwrap_err(),
                    DomainError::InvalidLatePolicy { field, value, max }
                );
            }
        }
    }

    #[test]
    fn missing_max_score_means_pass_fail() {
        assert_eq!(Grading::from_max_score(None).unwrap(), Grading::PassFail);
        assert_eq!(
            Grading::from_max_score(Some(5)).unwrap(),
            Grading::Score {
                max_score: MaxScore::new(5).unwrap()
            }
        );
    }

    #[test]
    fn closing_exactly_one_period_later_needs_one_period_and_a_moment_more_needs_two() {
        let policy = LatePolicy::new(7, 1, 4, DeductedFrom::WorkScore).unwrap();
        let deadline = t0();
        let week = Duration::weeks(1);

        assert_eq!(policy.periods_to_close_at(deadline, deadline), 0);
        assert_eq!(
            policy.periods_to_close_at(deadline, deadline + Duration::days(1)),
            1
        );
        assert_eq!(policy.periods_to_close_at(deadline, deadline + week), 1);
        assert_eq!(
            policy.periods_to_close_at(deadline, deadline + week + Duration::milliseconds(500)),
            2
        );
        assert_eq!(policy.periods_until(deadline, deadline + week), 2);
    }

    #[test]
    fn preview_uses_the_same_penalty_for_any_period_count() {
        let policy = LatePolicy::new(7, 1, 4, DeductedFrom::WorkScore).unwrap();
        let maximum = MaxScore::new(10).unwrap();
        assert_eq!(policy.possible_score(maximum, 0), 10);
        assert_eq!(policy.possible_score(maximum, 1), 9);
        assert_eq!(policy.possible_score(maximum, 4), 6);
        assert_eq!(policy.possible_score(maximum, u32::MAX), 0);
    }

    #[test]
    fn a_penalty_to_the_course_total_never_lowers_the_possible_work_score() {
        let policy = LatePolicy::new(7, 3, 4, DeductedFrom::CourseTotal).unwrap();
        let maximum = MaxScore::new(10).unwrap();

        for periods in [0, 1, 4, u32::MAX] {
            assert_eq!(policy.possible_score(maximum, periods), 10, "{periods}");
        }
        assert_eq!(policy.penalty_for(4), 12);
    }

    #[test]
    fn the_penalty_of_a_late_submission_carries_the_steps_points_and_deduction() {
        let policy = LatePolicy::new(7, 3, 4, DeductedFrom::CourseTotal).unwrap();

        assert_eq!(policy.late_penalty(0), None);
        let late = policy.late_penalty(2).unwrap();
        assert_eq!(late.periods(), 2);
        assert_eq!(late.points(), 6);
        assert_eq!(late.deducted_from(), DeductedFrom::CourseTotal);
        assert_eq!(policy.late_penalty(u32::MAX).unwrap().points(), MAX_SCORE);
    }

    #[test]
    fn reading_old_long_horizons_does_not_apply_the_new_input_limit() {
        let work = restore(AssignmentStatus::Draft, 2, None).unwrap();
        let restored = Assignment::restore(AssignmentParts {
            id: work.id,
            teaching_course_id: work.teaching_course_id,
            subject_id: work.subject_id,
            groups: work.groups.clone(),
            title: work.title.as_str().to_owned(),
            body: work.body.as_str().to_owned(),
            max_attempts: work.max_attempts,
            grading: Grading::scored(10).unwrap(),
            late_policy: Some(LatePolicy::new(365, 1, 100, DeductedFrom::WorkScore).unwrap()),
            makeup: None,
            status: work.status,
            created_by: work.created_by,
            created_at: work.created_at,
            deadline_at: work.deadline_at,
            published_at: work.published_at,
        })
        .unwrap();
        assert_eq!(
            restored.closes_at(),
            work.deadline_at + Duration::days(36_500)
        );
    }

    fn out_of(value: u16, max_score: u16) -> Option<ReviewScore> {
        Some(ReviewScore { value, max_score })
    }

    #[test]
    fn a_scored_assignment_requires_a_score_within_its_scale() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let scored = assignment(&course, &teacher).scored(5).build();

        for value in [0, 5] {
            assert_eq!(
                scored.regular_grade(out_of(value, 5), t0()),
                Ok(Some(Grade::new(value, 5, None)))
            );
        }
        assert_eq!(
            scored.regular_grade(out_of(6, 5), t0()),
            Err(DomainError::ScoreOutOfRange { value: 6, max: 5 })
        );
        assert_eq!(
            scored.regular_grade(None, t0()),
            Err(DomainError::ScoreRequired)
        );
    }

    #[test]
    fn a_score_given_out_of_another_maximum_is_refused_even_within_the_scale() {
        let teacher = teacher(1);
        let course = course_for(&student_group("test"), &teacher);
        let scored = assignment(&course, &teacher).scored(10).build();

        for given_max in [5, 20] {
            assert_eq!(
                scored.regular_grade(out_of(4, given_max), t0()),
                Err(DomainError::ScoreScaleMismatch { given_max, max: 10 })
            );
        }
    }

    #[test]
    fn pass_fail_grading_rejects_any_score() {
        let pass_fail = restore(AssignmentStatus::Published, 1, Some(t0())).unwrap();

        assert_eq!(pass_fail.regular_grade(None, t0()), Ok(None));
        assert_eq!(
            pass_fail.regular_grade(out_of(1, 1), t0()),
            Err(DomainError::ScoreNotAllowed)
        );
    }

    #[test]
    fn an_assignment_without_chosen_groups_is_issued_to_every_group_of_its_course() {
        let teacher = teacher(1);
        let (first, second) = (student_group("first"), student_group("second"));
        let course = teaching_course(&subject("Backend"), &[&first, &second], &teacher);

        let issued = assignment(&course, &teacher).build();

        assert_eq!(issued.groups(), course.group_ids());
        assert!(issued.is_issued_to(&student(2, &first)));
        assert!(issued.is_issued_to(&student(3, &second)));
    }

    #[test]
    fn an_assignment_keeps_the_chosen_part_of_the_course_groups() {
        let teacher = teacher(1);
        let (first, second) = (student_group("first"), student_group("second"));
        let course = teaching_course(&subject("Backend"), &[&first, &second], &teacher);

        let issued = assignment(&course, &teacher).groups([&second]).build();

        assert_eq!(issued.groups(), &BTreeSet::from([second.id()]));
        assert!(issued.is_issued_to(&student(2, &second)));
        assert!(!issued.is_issued_to(&student(3, &first)));
        assert!(!issued.is_issued_to(&teacher));
    }

    #[test]
    fn an_assignment_cannot_be_issued_to_no_group_or_a_group_outside_its_course() {
        let teacher = teacher(1);
        let (own, foreign) = (student_group("own"), student_group("foreign"));
        let course = course_for(&own, &teacher);

        assert_eq!(
            assignment(&course, &teacher).groups([]).try_build(),
            Err(DomainError::AssignmentHasNoGroups)
        );
        assert_eq!(
            assignment(&course, &teacher)
                .groups([&own, &foreign])
                .try_build(),
            Err(DomainError::AssignmentGroupOutsideCourse)
        );
    }

    #[test]
    fn changing_an_assignment_never_changes_the_groups_it_is_issued_to() {
        let teacher = teacher(1);
        let (first, second) = (student_group("first"), student_group("second"));
        let course = teaching_course(&subject("Backend"), &[&first, &second], &teacher);
        let mut issued = assignment(&course, &teacher).groups([&first]).build();
        let groups = issued.groups().clone();

        issued.publish(&course, &teacher, t0()).unwrap();
        issued
            .change_deadline(&course, &teacher, t0() + Duration::days(14))
            .unwrap();
        issued
            .change_grading(&course, &teacher, Grading::scored(10).unwrap(), None)
            .unwrap();
        issued.archive(&course, &teacher).unwrap();

        assert_eq!(issued.groups(), &groups);
    }

    #[test]
    fn restore_rejects_draft_assignment_with_published_at() {
        assert_eq!(
            restore(AssignmentStatus::Draft, 1, Some(t0())).unwrap_err(),
            DomainError::InvalidAssignmentPublishedState
        );
    }
}
