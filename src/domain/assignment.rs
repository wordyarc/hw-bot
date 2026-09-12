use chrono::{DateTime, Duration, Utc};

use crate::domain::{
    ASSIGNMENT_TITLE_MAX_LEN, AssignmentId, DomainError, DomainResult, NonEmptyString,
    StudentGroupId, SubjectId, TeachingCourse, TeachingCourseId, USER_TEXT_MAX_LEN, User, UserId,
};
use crate::string_enum::str_enum;

pub const MAX_ATTEMPTS: u16 = i16::MAX as u16;
pub const MAX_SCORE: u16 = 100;
pub const MAX_LATE_PERIOD_DAYS: u16 = 365;
pub const MAX_LATE_PENALTY: u16 = MAX_SCORE;
pub const MAX_LATE_PERIODS: u16 = 100;

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum AssignmentStatus {
        Draft => "draft",
        Published => "published",
        Archived => "archived",
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Grading {
    #[default]
    PassFail,
    Score {
        max_score: u16,
    },
}

impl Grading {
    pub fn scored(max_score: u16) -> DomainResult<Self> {
        if max_score == 0 || max_score > MAX_SCORE {
            return Err(DomainError::InvalidMaxScore {
                value: max_score,
                max: MAX_SCORE,
            });
        }

        Ok(Self::Score { max_score })
    }

    /// Отсутствие максимума - это «зачёт/незачёт», поэтому колонка в БД одна и nullable.
    pub fn from_max_score(max_score: Option<u16>) -> DomainResult<Self> {
        max_score.map_or(Ok(Self::PassFail), Self::scored)
    }

    pub fn max_score(self) -> Option<u16> {
        match self {
            Self::PassFail => None,
            Self::Score { max_score } => Some(max_score),
        }
    }

    pub fn is_scored(self) -> bool {
        matches!(self, Self::Score { .. })
    }

    pub fn validate_score(self, score: Option<u16>) -> DomainResult<Option<u16>> {
        match (self, score) {
            (Self::PassFail, None) => Ok(None),
            (Self::PassFail, Some(_)) => Err(DomainError::ScoreNotAllowed),
            (Self::Score { .. }, None) => Err(DomainError::ScoreRequired),
            (Self::Score { max_score }, Some(value)) if value > max_score => {
                Err(DomainError::ScoreOutOfRange {
                    value,
                    max: max_score,
                })
            }
            (Self::Score { .. }, Some(value)) => Ok(Some(value)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Assignment {
    id: AssignmentId,
    teaching_course_id: TeachingCourseId,
    subject_id: SubjectId,
    student_group_id: StudentGroupId,
    title: NonEmptyString,
    body: NonEmptyString,
    max_attempts: u16,
    grading: Grading,
    late_policy: Option<LatePolicy>,
    status: AssignmentStatus,
    created_by: UserId,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatePolicy {
    period_days: u16,
    penalty: u16,
    max_periods: u16,
}

impl LatePolicy {
    pub fn new(period_days: u16, penalty: u16, max_periods: u16) -> DomainResult<Self> {
        check_range("period_days", period_days, MAX_LATE_PERIOD_DAYS)?;
        check_range("penalty", penalty, MAX_LATE_PENALTY)?;
        check_range("max_periods", max_periods, MAX_LATE_PERIODS)?;

        Ok(Self {
            period_days,
            penalty,
            max_periods,
        })
    }

    pub fn period_days(self) -> u16 {
        self.period_days
    }

    pub fn penalty(self) -> u16 {
        self.penalty
    }

    pub fn max_periods(self) -> u16 {
        self.max_periods
    }

    fn late_periods(self, deadline_at: DateTime<Utc>, at: DateTime<Utc>) -> u16 {
        if at <= deadline_at {
            return 0;
        }

        let lateness = (at - deadline_at).num_seconds();
        let period = i64::from(self.period_days) * 24 * 60 * 60;
        let periods = (lateness + period - 1) / period;

        u16::try_from(periods).unwrap_or(u16::MAX)
    }

    fn closes_at(self, deadline_at: DateTime<Utc>) -> DateTime<Utc> {
        deadline_at + Duration::days(i64::from(self.period_days) * i64::from(self.max_periods))
    }

    /// Штраф выше максимума шкалы всё равно даёт ноль, переполнение зажимается
    fn penalty_for(self, late_periods: u16) -> u16 {
        late_periods.saturating_mul(self.penalty).min(MAX_SCORE)
    }
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
    pub student_group_id: StudentGroupId,
    pub title: String,
    pub body: String,
    pub max_attempts: u16,
    pub grading: Grading,
    pub late_policy: Option<LatePolicy>,
    pub status: AssignmentStatus,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub deadline_at: DateTime<Utc>,
    pub published_at: Option<DateTime<Utc>>,
}

impl Assignment {
    pub(crate) fn restore(parts: AssignmentParts) -> DomainResult<Self> {
        if parts.max_attempts == 0 || parts.max_attempts > MAX_ATTEMPTS {
            return Err(DomainError::InvalidAttemptLimit {
                value: parts.max_attempts,
            });
        }

        if parts.deadline_at <= parts.created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }

        // Штраф выражен в баллах, поэтому у «зачёт/незачёт» политика опозданий смысла не имеет.
        if parts.late_policy.is_some() && !parts.grading.is_scored() {
            return Err(DomainError::LatePolicyRequiresScoredGrading);
        }

        // В архив можно убрать и черновик, поэтому published_at у архива необязателен.
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
            student_group_id: parts.student_group_id,
            title: NonEmptyString::new_bounded(
                "assignment.title",
                parts.title,
                ASSIGNMENT_TITLE_MAX_LEN,
            )?,
            body: NonEmptyString::new_bounded("assignment.body", parts.body, USER_TEXT_MAX_LEN)?,
            max_attempts: parts.max_attempts,
            grading: parts.grading,
            late_policy: parts.late_policy,
            status: parts.status,
            created_by: parts.created_by,
            created_at: parts.created_at,
            deadline_at: parts.deadline_at,
            published_at: parts.published_at,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AssignmentId,
        teaching_course: &TeachingCourse,
        created_by: &User,
        title: impl Into<String>,
        body: impl Into<String>,
        max_attempts: u16,
        grading: Grading,
        late_policy: Option<LatePolicy>,
        created_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !teaching_course.can_be_managed_by(created_by) {
            return Err(DomainError::PermissionDenied {
                action: "create assignment",
            });
        }

        Self::restore(AssignmentParts {
            id,
            teaching_course_id: teaching_course.id(),
            subject_id: teaching_course.subject_id(),
            student_group_id: teaching_course.student_group_id(),
            title: title.into(),
            body: body.into(),
            max_attempts,
            grading,
            late_policy,
            status: AssignmentStatus::Draft,
            created_by: created_by.id(),
            created_at,
            deadline_at,
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
        self.ensure_managed_by(teaching_course, actor, "change assignment deadline")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentIsArchived);
        }

        if new_deadline_at <= self.created_at {
            return Err(DomainError::AssignmentDeadlineMustBeAfterCreatedAt);
        }

        self.deadline_at = new_deadline_at;

        Ok(())
    }

    pub fn change_grading(
        &mut self,
        teaching_course: &TeachingCourse,
        actor: &User,
        grading: Grading,
        late_policy: Option<LatePolicy>,
    ) -> DomainResult<()> {
        self.ensure_managed_by(teaching_course, actor, "change assignment grading")?;

        if self.status == AssignmentStatus::Archived {
            return Err(DomainError::AssignmentIsArchived);
        }

        if late_policy.is_some() && !grading.is_scored() {
            return Err(DomainError::LatePolicyRequiresScoredGrading);
        }

        self.grading = grading;
        self.late_policy = late_policy;

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

    pub fn student_group_id(&self) -> StudentGroupId {
        self.student_group_id
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

    pub fn closes_at(&self) -> DateTime<Utc> {
        self.late_policy.map_or(self.deadline_at, |policy| {
            policy.closes_at(self.deadline_at)
        })
    }

    pub fn late_periods_at(&self, at: DateTime<Utc>) -> u16 {
        self.late_policy
            .map_or(0, |policy| policy.late_periods(self.deadline_at, at))
    }

    pub fn penalty_at(&self, at: DateTime<Utc>) -> u16 {
        self.late_policy
            .map_or(0, |policy| policy.penalty_for(self.late_periods_at(at)))
    }

    pub fn ensure_open_for_submission(&self, at: DateTime<Utc>) -> DomainResult<()> {
        let closes_at = self.closes_at();
        if at <= closes_at {
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
    use crate::test_support::t0;

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
            student_group_id: StudentGroupId::new(),
            title: "Rust ownership".to_owned(),
            body: "Send a pull request link.".to_owned(),
            max_attempts,
            grading: Grading::PassFail,
            late_policy: None,
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
            ("penalty", MAX_LATE_PENALTY),
            ("max_periods", MAX_LATE_PERIODS),
        ] {
            for value in [0, max + 1] {
                let policy = match field {
                    "period_days" => LatePolicy::new(value, 1, 1),
                    "penalty" => LatePolicy::new(1, value, 1),
                    _ => LatePolicy::new(1, 1, value),
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
            Grading::Score { max_score: 5 }
        );
    }

    #[test]
    fn scored_grading_requires_a_score_within_the_scale() {
        let grading = Grading::scored(5).unwrap();

        assert_eq!(grading.validate_score(Some(0)).unwrap(), Some(0));
        assert_eq!(grading.validate_score(Some(5)).unwrap(), Some(5));
        assert_eq!(
            grading.validate_score(Some(6)).unwrap_err(),
            DomainError::ScoreOutOfRange { value: 6, max: 5 }
        );
        assert_eq!(
            grading.validate_score(None).unwrap_err(),
            DomainError::ScoreRequired
        );
    }

    #[test]
    fn pass_fail_grading_rejects_any_score() {
        assert_eq!(Grading::PassFail.validate_score(None).unwrap(), None);
        assert_eq!(
            Grading::PassFail.validate_score(Some(1)).unwrap_err(),
            DomainError::ScoreNotAllowed
        );
    }

    #[test]
    fn restore_rejects_draft_assignment_with_published_at() {
        assert_eq!(
            restore(AssignmentStatus::Draft, 1, Some(t0())).unwrap_err(),
            DomainError::InvalidAssignmentPublishedState
        );
    }
}
