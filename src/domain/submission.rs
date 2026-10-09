use chrono::{DateTime, Utc};

use crate::domain::{
    Acceptance, Assignment, AssignmentId, DeductedFrom, DomainError, DomainResult, LatePenalty,
    MAX_ATTEMPTS, MAX_SCORE, MakeupRequest, NonEmptyString, Offer, SubmissionId, TeachingCourse,
    USER_TEXT_MAX_LEN, User, UserId,
};
use crate::string_enum::str_enum;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SubmissionHistory {
    used_attempts: u16,
    extra_attempts: u16,
    submission_count: u16,
    has_accepted_regular: bool,
    accepted_regular_score: Option<u16>,
    has_accepted_makeup: bool,
    has_pending_submission: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SubmissionHistoryParts {
    pub used_attempts: u16,
    pub extra_attempts: u16,
    pub submission_count: u16,
    pub has_accepted_regular: bool,
    pub accepted_regular_score: Option<u16>,
    pub has_accepted_makeup: bool,
    pub has_pending_submission: bool,
}

impl SubmissionHistory {
    pub(crate) fn restore(parts: SubmissionHistoryParts) -> Self {
        Self {
            used_attempts: parts.used_attempts,
            extra_attempts: parts.extra_attempts,
            submission_count: parts.submission_count,
            has_accepted_regular: parts.has_accepted_regular,
            accepted_regular_score: parts.accepted_regular_score,
            has_accepted_makeup: parts.has_accepted_makeup,
            has_pending_submission: parts.has_pending_submission,
        }
    }

    pub fn used_attempts(self) -> u16 {
        self.used_attempts
    }

    pub fn extra_attempts(self) -> u16 {
        self.extra_attempts
    }

    pub fn has_accepted_submission(self) -> bool {
        self.has_accepted_regular || self.has_accepted_makeup
    }

    pub fn has_pending_submission(self) -> bool {
        self.has_pending_submission
    }

    pub fn allowed_attempts_for(self, assignment: &Assignment) -> u16 {
        assignment.max_attempts() + self.extra_attempts
    }

    /// Даже с оставшимися попытками нельзя пересдать принятую или ещё не проверенную работу.
    pub fn remaining_attempts_for(self, assignment: &Assignment) -> u16 {
        self.allowed_attempts_for(assignment)
            .saturating_sub(self.used_attempts)
    }

    pub fn accepts_new_attempt_for(self, assignment: &Assignment, at: DateTime<Utc>) -> bool {
        self.ensure_regular_attempt(assignment, at).is_ok()
    }

    pub fn ensure_regular_attempt(
        self,
        assignment: &Assignment,
        at: DateTime<Utc>,
    ) -> DomainResult<()> {
        if !assignment.is_published() {
            return Err(DomainError::AssignmentIsNotPublished);
        }
        if self.has_pending_submission {
            return Err(DomainError::SubmissionAlreadyPendingReview);
        }
        self.ensure_admits(assignment, Acceptance::Regular)?;
        assignment.ensure_open_for_submission(at)
    }

    pub fn offer_at(
        self,
        assignment: &Assignment,
        at: DateTime<Utc>,
        makeup: MakeupRequest,
    ) -> Option<Offer> {
        if !assignment.is_published() {
            return None;
        }
        let acceptance = assignment
            .acceptance_at(at, makeup.consented_course())
            .ok()?;
        self.ensure_admits(assignment, acceptance).ok()?;

        Some(match acceptance {
            Acceptance::Regular => Offer::Regular,
            Acceptance::Makeup { credit } => Offer::Makeup {
                credit,
                until: makeup.course?.until(),
                balance: makeup.balance,
            },
        })
    }

    fn ensure_admits(self, assignment: &Assignment, acceptance: Acceptance) -> DomainResult<()> {
        if self.closed_for(acceptance) {
            return Err(DomainError::SubmissionAlreadyAccepted);
        }

        if self.has_pending_submission {
            return Err(DomainError::SubmissionAlreadyPendingReview);
        }

        if acceptance == Acceptance::Regular {
            let allowed = self.allowed_attempts_for(assignment);
            if self.used_attempts >= allowed {
                return Err(DomainError::AttemptsExhausted {
                    allowed,
                    used: self.used_attempts,
                });
            }
        }

        Ok(())
    }

    fn closed_for(self, acceptance: Acceptance) -> bool {
        match acceptance {
            Acceptance::Regular => self.has_accepted_submission(),
            Acceptance::Makeup { credit } => {
                self.has_accepted_makeup
                    || (self.has_accepted_regular
                        && credit.is_none_or(|credit| {
                            self.accepted_regular_score
                                .is_none_or(|score| score >= credit.get())
                        }))
            }
        }
    }

    fn next_attempt_number(self) -> DomainResult<u16> {
        self.submission_count
            .checked_add(1)
            .filter(|number| *number <= MAX_ATTEMPTS)
            .ok_or(DomainError::AttemptLimitExceeded {
                value: u32::from(self.submission_count) + 1,
                max: MAX_ATTEMPTS,
            })
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SubmissionStatus {
        PendingReview => "pending_review",
        Accepted => "accepted",
        Rejected => "rejected",
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SubmissionKind {
        Regular => "regular",
        Makeup => "makeup",
    }
}

str_enum! {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ReviewDecision {
        Accept => "accept",
        Reject => "reject",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grade {
    raw_score: u16,
    max_score: u16,
    late: Option<LatePenalty>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct GradeParts {
    pub raw_score: Option<u16>,
    pub max_score: Option<u16>,
    pub late_periods: Option<u32>,
    pub penalty: Option<u16>,
    pub deducted_from: Option<DeductedFrom>,
    pub work_score: Option<u16>,
    pub course_total_penalty: Option<u16>,
}

impl Grade {
    pub(crate) fn new(raw_score: u16, max_score: u16, late: Option<LatePenalty>) -> Self {
        Self {
            raw_score,
            max_score,
            late,
        }
    }

    pub(crate) fn restore(parts: GradeParts) -> DomainResult<Option<Self>> {
        let invalid = || DomainError::InvalidSubmissionGradeState;
        let (raw_score, max_score, late_periods, penalty, work_score, course_total_penalty) = match (
            parts.raw_score,
            parts.max_score,
            parts.late_periods,
            parts.penalty,
            parts.work_score,
            parts.course_total_penalty,
        ) {
            (None, None, None, None, None, None) if parts.deducted_from.is_none() => {
                return Ok(None);
            }
            (
                Some(raw_score),
                Some(max_score),
                Some(late_periods),
                Some(penalty),
                Some(work_score),
                Some(course_total_penalty),
            ) => (
                raw_score,
                max_score,
                late_periods,
                penalty,
                work_score,
                course_total_penalty,
            ),
            _ => return Err(invalid()),
        };
        let late = match (late_periods, penalty, parts.deducted_from) {
            (0, 0, None) => None,
            (periods, points, Some(deducted_from)) => {
                Some(LatePenalty::restore(periods, points, deducted_from).ok_or_else(invalid)?)
            }
            _ => return Err(invalid()),
        };
        let grade = Self::new(raw_score, max_score, late);
        if max_score == 0
            || max_score > MAX_SCORE
            || raw_score > max_score
            || grade.work_score() != work_score
            || grade.course_total_penalty() != course_total_penalty
        {
            return Err(invalid());
        }

        Ok(Some(grade))
    }

    pub fn raw_score(self) -> u16 {
        self.raw_score
    }

    pub fn max_score(self) -> u16 {
        self.max_score
    }

    pub fn late(self) -> Option<LatePenalty> {
        self.late
    }

    pub fn late_periods(self) -> u32 {
        self.late.map_or(0, LatePenalty::periods)
    }

    pub fn penalty(self) -> u16 {
        self.late.map_or(0, LatePenalty::points)
    }

    pub fn deducted_from(self) -> Option<DeductedFrom> {
        self.late.map(LatePenalty::deducted_from)
    }

    pub fn work_score(self) -> u16 {
        match self.late {
            Some(late) if late.deducted_from() == DeductedFrom::WorkScore => {
                self.raw_score.saturating_sub(late.points())
            }
            _ => self.raw_score,
        }
    }

    pub fn course_total_penalty(self) -> u16 {
        match self.late {
            Some(late) if late.deducted_from() == DeductedFrom::CourseTotal => late.points(),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod grade_tests {
    use super::*;
    use crate::domain::DeductedFrom::{CourseTotal, WorkScore};
    use crate::domain::LatePolicy;

    fn stored(
        raw_score: u16,
        late_periods: u32,
        penalty: u16,
        deducted_from: Option<DeductedFrom>,
        work_score: u16,
        course_total_penalty: u16,
    ) -> GradeParts {
        GradeParts {
            raw_score: Some(raw_score),
            max_score: Some(5),
            late_periods: Some(late_periods),
            penalty: Some(penalty),
            deducted_from,
            work_score: Some(work_score),
            course_total_penalty: Some(course_total_penalty),
        }
    }

    #[test]
    fn the_frozen_deduction_decides_where_the_effective_penalty_goes() {
        let late = |penalty_per_period, periods, deducted_from| {
            LatePolicy::new(7, penalty_per_period, 100, deducted_from)
                .unwrap()
                .late_penalty(periods)
        };

        for (raw_score, max_score, late, expected) in [
            (8, 10, None, (8, 0)),
            (8, 10, late(4, 1, WorkScore), (4, 0)),
            (8, 10, late(4, 1, CourseTotal), (8, 4)),
            (4, 5, late(5, 1, WorkScore), (0, 0)),
            (4, 5, late(5, 1, CourseTotal), (4, 5)),
            (100, 100, late(60, 2, WorkScore), (0, 0)),
            (100, 100, late(60, 2, CourseTotal), (100, 100)),
        ] {
            let grade = Grade::new(raw_score, max_score, late);

            assert_eq!(
                (grade.work_score(), grade.course_total_penalty()),
                expected,
                "{raw_score}/{max_score} {late:?}"
            );
            assert_eq!(grade.deducted_from(), late.map(LatePenalty::deducted_from));
        }
    }

    #[test]
    fn restores_both_deductions_and_a_grade_without_lateness() {
        for (parts, deducted_from) in [
            (stored(4, 0, 0, None, 4, 0), None),
            (stored(4, 2, 2, Some(WorkScore), 2, 0), Some(WorkScore)),
            (stored(4, 2, 5, Some(CourseTotal), 4, 5), Some(CourseTotal)),
            (stored(4, 2, 100, Some(WorkScore), 0, 0), Some(WorkScore)),
        ] {
            let grade = Grade::restore(parts).unwrap().unwrap();
            assert_eq!(grade.deducted_from(), deducted_from);
            assert_eq!(Some(grade.work_score()), parts.work_score);
            assert_eq!(
                Some(grade.course_total_penalty()),
                parts.course_total_penalty
            );
        }
        assert_eq!(Grade::restore(GradeParts::default()), Ok(None));
    }

    #[test]
    fn rejects_a_deduction_without_lateness_and_lateness_without_a_deduction() {
        for parts in [
            stored(4, 0, 0, Some(WorkScore), 4, 0),
            stored(4, 0, 0, Some(CourseTotal), 4, 0),
            stored(4, 2, 2, None, 2, 0),
            stored(4, 2, 2, None, 4, 0),
            stored(4, 2, 0, Some(WorkScore), 4, 0),
            stored(4, 0, 2, Some(WorkScore), 2, 0),
            GradeParts {
                deducted_from: Some(WorkScore),
                ..GradeParts::default()
            },
        ] {
            assert_eq!(
                Grade::restore(parts),
                Err(DomainError::InvalidSubmissionGradeState),
                "{parts:?}"
            );
        }
    }

    #[test]
    fn rejects_a_work_score_or_course_total_penalty_inconsistent_with_the_deduction() {
        for parts in [
            stored(4, 2, 2, Some(WorkScore), 4, 0),
            stored(4, 2, 2, Some(WorkScore), 2, 2),
            stored(4, 2, 2, Some(CourseTotal), 2, 2),
            stored(4, 2, 2, Some(CourseTotal), 4, 0),
            stored(4, 0, 0, None, 3, 0),
            stored(4, 0, 0, None, 4, 1),
        ] {
            assert_eq!(
                Grade::restore(parts),
                Err(DomainError::InvalidSubmissionGradeState),
                "{parts:?}"
            );
        }
    }

    #[test]
    fn rejects_a_stored_penalty_above_the_effective_limit() {
        assert_eq!(
            Grade::restore(stored(4, 2, 101, Some(WorkScore), 0, 0)),
            Err(DomainError::InvalidSubmissionGradeState)
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Submission {
    id: SubmissionId,
    assignment_id: AssignmentId,
    student_id: UserId,
    kind: SubmissionKind,
    attempt_number: u16,
    content: NonEmptyString,
    status: SubmissionStatus,
    submitted_at: DateTime<Utc>,
    reviewed_by: Option<UserId>,
    reviewed_at: Option<DateTime<Utc>>,
    review_comment: Option<NonEmptyString>,
    grade: Option<Grade>,
}

pub(crate) struct SubmissionParts {
    pub id: SubmissionId,
    pub assignment_id: AssignmentId,
    pub student_id: UserId,
    pub kind: SubmissionKind,
    pub attempt_number: u16,
    pub content: String,
    pub status: SubmissionStatus,
    pub submitted_at: DateTime<Utc>,
    pub reviewed_by: Option<UserId>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub review_comment: Option<String>,
    pub grade: GradeParts,
}

impl Submission {
    pub(crate) fn restore(parts: SubmissionParts) -> DomainResult<Self> {
        if parts.attempt_number == 0 {
            return Err(DomainError::InvalidSubmissionAttemptNumber {
                value: parts.attempt_number,
            });
        }

        match parts.status {
            SubmissionStatus::PendingReview
                if parts.reviewed_by.is_some()
                    || parts.reviewed_at.is_some()
                    || parts.review_comment.is_some() =>
            {
                return Err(DomainError::InvalidSubmissionReviewState);
            }
            SubmissionStatus::Accepted | SubmissionStatus::Rejected
                if parts.reviewed_by.is_none()
                    || parts.reviewed_at.is_none()
                    || parts.review_comment.is_none() =>
            {
                return Err(DomainError::InvalidSubmissionReviewState);
            }
            _ => {}
        }

        let grade = Grade::restore(parts.grade)?;
        if grade.is_some() && parts.status != SubmissionStatus::Accepted {
            return Err(DomainError::InvalidSubmissionGradeState);
        }
        if parts.kind == SubmissionKind::Makeup && grade.and_then(Grade::late).is_some() {
            return Err(DomainError::InvalidSubmissionGradeState);
        }

        Ok(Self {
            id: parts.id,
            assignment_id: parts.assignment_id,
            student_id: parts.student_id,
            kind: parts.kind,
            attempt_number: parts.attempt_number,
            content: NonEmptyString::new_bounded(
                "submission.content",
                parts.content,
                USER_TEXT_MAX_LEN,
            )?,
            status: parts.status,
            submitted_at: parts.submitted_at,
            reviewed_by: parts.reviewed_by,
            reviewed_at: parts.reviewed_at,
            review_comment: parts
                .review_comment
                .map(|comment| {
                    NonEmptyString::new_bounded(
                        "submission.review_comment",
                        comment,
                        USER_TEXT_MAX_LEN,
                    )
                })
                .transpose()?,
            grade,
        })
    }

    pub fn admit(
        assignment: &Assignment,
        student: &User,
        history: SubmissionHistory,
        submitted_at: DateTime<Utc>,
        makeup: MakeupRequest,
    ) -> DomainResult<Acceptance> {
        if !student.role().can_submit_homework() {
            return Err(DomainError::PermissionDenied {
                action: "submit homework",
            });
        }

        if !assignment.is_published() {
            return Err(DomainError::AssignmentIsNotPublished);
        }

        let acceptance = assignment.acceptance_at(submitted_at, makeup.consented_course())?;

        if student.student_group_id() != Some(assignment.student_group_id()) {
            return Err(DomainError::StudentDoesNotBelongToAssignmentGroup);
        }

        history.ensure_admits(assignment, acceptance)?;

        Ok(acceptance)
    }

    pub fn submit(
        id: SubmissionId,
        assignment: &Assignment,
        student: &User,
        history: SubmissionHistory,
        content: impl Into<String>,
        submitted_at: DateTime<Utc>,
        makeup: MakeupRequest,
    ) -> DomainResult<Self> {
        let acceptance = Self::admit(assignment, student, history, submitted_at, makeup)?;
        if acceptance != Acceptance::Regular {
            makeup.balance.ensure_available()?;
        }

        Self::restore(SubmissionParts {
            id,
            assignment_id: assignment.id(),
            student_id: student.id(),
            kind: acceptance.kind(),
            attempt_number: history.next_attempt_number()?,
            content: content.into(),
            status: SubmissionStatus::PendingReview,
            submitted_at,
            reviewed_by: None,
            reviewed_at: None,
            review_comment: None,
            grade: GradeParts::default(),
        })
    }

    pub fn accept(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        score: Option<u16>,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            teaching_course,
            reviewer,
            SubmissionStatus::Accepted,
            score,
            comment,
            reviewed_at,
        )
    }

    pub fn reject(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        self.complete_review(
            assignment,
            teaching_course,
            reviewer,
            SubmissionStatus::Rejected,
            None,
            comment,
            reviewed_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_review(
        &mut self,
        assignment: &Assignment,
        teaching_course: &TeachingCourse,
        reviewer: &User,
        status: SubmissionStatus,
        score: Option<u16>,
        comment: impl Into<String>,
        reviewed_at: DateTime<Utc>,
    ) -> DomainResult<()> {
        if self.assignment_id != assignment.id() {
            return Err(DomainError::SubmissionDoesNotBelongToAssignment);
        }

        assignment.ensure_belongs_to_course(teaching_course)?;

        if !teaching_course.can_be_managed_by(reviewer) {
            return Err(DomainError::PermissionDenied {
                action: "review homework",
            });
        }

        if self.status != SubmissionStatus::PendingReview {
            return Err(DomainError::SubmissionIsNotPendingReview);
        }

        let grade = match (status, self.kind) {
            (SubmissionStatus::Accepted, SubmissionKind::Regular) => {
                // Берём время отправки, чтобы длительность проверки не влияла на штраф студенту.
                let late = assignment.late_penalty_at(self.submitted_at);
                let max_score = assignment.grading().max_score();

                assignment
                    .grading()
                    .validate_score(score)?
                    .map(|raw_score| {
                        Grade::new(
                            raw_score,
                            max_score.expect("a validated score requires scored grading"),
                            late,
                        )
                    })
            }
            (SubmissionStatus::Accepted, SubmissionKind::Makeup) => {
                if score.is_some() {
                    return Err(DomainError::ScoreNotAllowed);
                }
                assignment.makeup_grade()?
            }
            _ => None,
        };

        let review_comment =
            NonEmptyString::new_bounded("submission.review_comment", comment, USER_TEXT_MAX_LEN)?;

        self.status = status;
        self.reviewed_by = Some(reviewer.id());
        self.reviewed_at = Some(reviewed_at);
        self.review_comment = Some(review_comment);
        self.grade = grade;

        Ok(())
    }

    pub fn id(&self) -> SubmissionId {
        self.id
    }

    pub fn assignment_id(&self) -> AssignmentId {
        self.assignment_id
    }

    pub fn student_id(&self) -> UserId {
        self.student_id
    }

    pub fn kind(&self) -> SubmissionKind {
        self.kind
    }

    pub fn attempt_number(&self) -> u16 {
        self.attempt_number
    }

    pub fn content(&self) -> &NonEmptyString {
        &self.content
    }

    pub fn status(&self) -> SubmissionStatus {
        self.status
    }

    pub fn submitted_at(&self) -> DateTime<Utc> {
        self.submitted_at
    }

    pub fn reviewed_by(&self) -> Option<UserId> {
        self.reviewed_by
    }

    pub fn reviewed_at(&self) -> Option<DateTime<Utc>> {
        self.reviewed_at
    }

    pub fn review_comment(&self) -> Option<&NonEmptyString> {
        self.review_comment.as_ref()
    }

    pub fn grade(&self) -> Option<Grade> {
        self.grade
    }

    pub fn is_pending_review(&self) -> bool {
        self.status == SubmissionStatus::PendingReview
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::t0;

    fn pending_parts() -> SubmissionParts {
        SubmissionParts {
            id: SubmissionId::new(),
            assignment_id: AssignmentId::new(),
            student_id: UserId::new(),
            kind: SubmissionKind::Regular,
            attempt_number: 1,
            content: "https://github.com/course/hw/pull/1".to_owned(),
            status: SubmissionStatus::PendingReview,
            submitted_at: t0(),
            reviewed_by: None,
            reviewed_at: None,
            review_comment: None,
            grade: GradeParts::default(),
        }
    }

    fn accepted_parts() -> SubmissionParts {
        SubmissionParts {
            status: SubmissionStatus::Accepted,
            reviewed_by: Some(UserId::new()),
            reviewed_at: Some(t0()),
            review_comment: Some("Good".to_owned()),
            ..pending_parts()
        }
    }

    fn grade(raw_score: u16, max_score: u16) -> GradeParts {
        GradeParts {
            raw_score: Some(raw_score),
            max_score: Some(max_score),
            late_periods: Some(2),
            penalty: Some(2),
            deducted_from: Some(DeductedFrom::WorkScore),
            work_score: Some(raw_score.saturating_sub(2)),
            course_total_penalty: Some(0),
        }
    }

    #[test]
    fn restore_accepts_pending_submission_without_review_fields() {
        let submission = Submission::restore(pending_parts()).unwrap();

        assert_eq!(submission.status(), SubmissionStatus::PendingReview);
        assert_eq!(submission.attempt_number(), 1);
    }

    #[test]
    fn restore_rejects_zero_attempt_number() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                attempt_number: 0,
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionAttemptNumber { value: 0 }
        );
    }

    #[test]
    fn restore_requires_review_fields_for_completed_submission() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                status: SubmissionStatus::Accepted,
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionReviewState
        );
    }

    #[test]
    fn restore_rejects_partial_grade() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                grade: GradeParts {
                    raw_score: Some(4),
                    ..GradeParts::default()
                },
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_rejects_grade_on_submission_that_is_not_accepted() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                status: SubmissionStatus::Rejected,
                grade: grade(4, 5),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_rejects_raw_score_above_the_frozen_scale() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                grade: grade(4, 3),
                ..accepted_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
    }

    #[test]
    fn restore_accepts_full_grade_of_accepted_submission() {
        let grade = Submission::restore(SubmissionParts {
            grade: grade(4, 5),
            ..accepted_parts()
        })
        .unwrap()
        .grade()
        .unwrap();

        assert_eq!(grade.raw_score(), 4);
        assert_eq!(grade.max_score(), 5);
        assert_eq!(grade.late_periods(), 2);
        assert_eq!(grade.penalty(), 2);
        assert_eq!(grade.deducted_from(), Some(DeductedFrom::WorkScore));
        assert_eq!(grade.work_score(), 2);
        assert_eq!(grade.course_total_penalty(), 0);
    }

    #[test]
    fn restore_rejects_a_late_grade_of_a_makeup() {
        let makeup = |grade| SubmissionParts {
            kind: SubmissionKind::Makeup,
            grade,
            ..accepted_parts()
        };

        assert_eq!(
            Submission::restore(makeup(grade(4, 5))).unwrap_err(),
            DomainError::InvalidSubmissionGradeState
        );
        let on_time = GradeParts {
            late_periods: Some(0),
            penalty: Some(0),
            deducted_from: None,
            work_score: Some(4),
            ..grade(4, 5)
        };
        assert_eq!(
            Submission::restore(makeup(on_time)).unwrap().kind(),
            SubmissionKind::Makeup
        );
    }

    #[test]
    fn restore_rejects_review_fields_for_pending_submission() {
        assert_eq!(
            Submission::restore(SubmissionParts {
                reviewed_by: Some(UserId::new()),
                reviewed_at: Some(t0()),
                review_comment: Some("review".to_owned()),
                ..pending_parts()
            })
            .unwrap_err(),
            DomainError::InvalidSubmissionReviewState
        );
    }
}
