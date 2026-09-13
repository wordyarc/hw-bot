use std::collections::HashMap;

use crate::pagination::PageRequest;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::ensure_updated;
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, DomainError, Role, StudentGroupId, Submission, SubmissionHistory, SubmissionId,
    SubmissionParts, SubmissionStatus, TeachingCourseId, User, UserId,
};
use crate::read_models::{AssignmentGrade, QueuePlace};

const ATTEMPT_NUMBER: &str = "submissions.attempt_number";
const STATUS: &str = "submissions.status";
const RAW_SCORE: &str = "submissions.raw_score";
const GRADE_MAX_SCORE: &str = "submissions.grade_max_score";
const LATE_PERIODS: &str = "submissions.late_periods";
const PENALTY: &str = "submissions.penalty";
const FINAL_SCORE: &str = "submissions.final_score";
const MAX_SCORE: &str = "assignments.max_score";

/// Оценка пишется и читается только целиком.
struct GradeColumns {
    raw_score: Option<i16>,
    max_score: Option<i16>,
    late_periods: Option<i32>,
    penalty: Option<i16>,
    final_score: Option<i16>,
}

impl GradeColumns {
    fn of(submission: &Submission) -> DbResult<Self> {
        let Some(grade) = submission.grade() else {
            return Ok(Self {
                raw_score: None,
                max_score: None,
                late_periods: None,
                penalty: None,
                final_score: None,
            });
        };

        Ok(Self {
            raw_score: Some(checked_num_cast(RAW_SCORE, grade.raw_score())?),
            max_score: Some(checked_num_cast(GRADE_MAX_SCORE, grade.max_score())?),
            late_periods: Some(checked_num_cast(LATE_PERIODS, grade.late_periods())?),
            penalty: Some(checked_num_cast(PENALTY, grade.penalty())?),
            final_score: Some(checked_num_cast(FINAL_SCORE, grade.final_score())?),
        })
    }
}

macro_rules! select_submissions {
    ($tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query_as!(
            SubmissionRow,
            r#"
            select
                id, assignment_id, student_id, attempt_number, content, status, submitted_at,
                reviewed_by, reviewed_at, review_comment, raw_score, grade_max_score, late_periods,
                penalty, final_score
            from submissions
            "# + $tail
            $(, $arg)*
        )
    };
}

pub async fn find_by_id<'e, E>(executor: E, id: SubmissionId) -> DbResult<Option<Submission>>
where
    E: PgExecutor<'e>,
{
    let row = select_submissions!("where id = $1", id.as_uuid())
        .fetch_optional(executor)
        .await?;

    row.map(Submission::try_from).transpose()
}

pub async fn insert<'e, E>(executor: E, submission: &Submission) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let grade = GradeColumns::of(submission)?;

    sqlx::query!(
        r#"
        insert into submissions (
            id, assignment_id, student_id, attempt_number, content, status, submitted_at,
            reviewed_by, reviewed_at, review_comment, raw_score, grade_max_score, late_periods,
            penalty, final_score
        )
        values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        "#,
        submission.id().as_uuid(),
        submission.assignment_id().as_uuid(),
        submission.student_id().as_uuid(),
        checked_num_cast::<_, i16>(ATTEMPT_NUMBER, submission.attempt_number())?,
        submission.content().as_str(),
        submission.status().as_str(),
        submission.submitted_at(),
        submission.reviewed_by().map(UserId::as_uuid),
        submission.reviewed_at(),
        submission.review_comment().map(|comment| comment.as_str()),
        grade.raw_score,
        grade.max_score,
        grade.late_periods,
        grade.penalty,
        grade.final_score,
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn save_review_result<'e, E>(executor: E, submission: &Submission) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    if submission.status() == SubmissionStatus::PendingReview {
        return Err(DbError::Conflict {
            message: "pending submission cannot be saved as a review result",
        });
    }

    let grade = GradeColumns::of(submission)?;

    let result = sqlx::query!(
        r#"
        update submissions
        set
            status = $2,
            reviewed_by = $3,
            reviewed_at = $4,
            review_comment = $5,
            raw_score = $6,
            grade_max_score = $7,
            late_periods = $8,
            penalty = $9,
            final_score = $10
        where id = $1
          and status = 'pending_review'
        "#,
        submission.id().as_uuid(),
        submission.status().as_str(),
        submission.reviewed_by().map(UserId::as_uuid),
        submission.reviewed_at(),
        submission.review_comment().map(|comment| comment.as_str()),
        grade.raw_score,
        grade.max_score,
        grade.late_periods,
        grade.penalty,
        grade.final_score,
    )
    .execute(executor)
    .await?;

    ensure_updated(
        result,
        DbError::RuleViolation(DomainError::SubmissionIsNotPendingReview),
    )
}

pub async fn load_history<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    student_id: UserId,
) -> DbResult<SubmissionHistory>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query!(
        r#"
        with history as (
            select
                count(*) filter (
                    where status in ('accepted', 'rejected')
                )::bigint as used_attempts,
                coalesce(bool_or(status = 'accepted'), false) as has_accepted_submission,
                coalesce(bool_or(status = 'pending_review'), false) as has_pending_submission
            from submissions
            where assignment_id = $1
              and student_id = $2
        ),
        progress as (
            select coalesce(max(extra_attempts), 0)::smallint as extra_attempts
            from student_assignment_progress
            where assignment_id = $1
              and student_id = $2
        )
        select
            history.used_attempts as "used_attempts!",
            progress.extra_attempts as "extra_attempts!",
            history.has_accepted_submission as "has_accepted_submission!",
            history.has_pending_submission as "has_pending_submission!"
        from history
        cross join progress
        "#,
        assignment_id.as_uuid(),
        student_id.as_uuid(),
    )
    .fetch_one(executor)
    .await?;

    Ok(SubmissionHistory::new(
        checked_num_cast("submission_history.used_attempts", row.used_attempts)?,
        checked_num_cast("submission_history.extra_attempts", row.extra_attempts)?,
        row.has_accepted_submission,
        row.has_pending_submission,
    ))
}

pub async fn load_histories<'e, E>(
    executor: E,
    assignment_ids: &[Uuid],
    student_id: UserId,
) -> DbResult<HashMap<AssignmentId, SubmissionHistory>>
where
    E: PgExecutor<'e>,
{
    let rows = sqlx::query!(
        r#"
        with history as (
            select
                assignment_id,
                count(*) filter (
                    where status in ('accepted', 'rejected')
                )::bigint as used_attempts,
                bool_or(status = 'accepted') as has_accepted_submission,
                bool_or(status = 'pending_review') as has_pending_submission
            from submissions
            where student_id = $2
              and assignment_id = any($1)
            group by assignment_id
        )
        select
            requested.assignment_id as "assignment_id!",
            coalesce(history.used_attempts, 0) as "used_attempts!",
            coalesce(progress.extra_attempts, 0)::smallint as "extra_attempts!",
            coalesce(history.has_accepted_submission, false) as "has_accepted_submission!",
            coalesce(history.has_pending_submission, false) as "has_pending_submission!"
        from unnest($1::uuid[]) as requested (assignment_id)
        left join history
          on history.assignment_id = requested.assignment_id
        left join student_assignment_progress progress
          on progress.assignment_id = requested.assignment_id
         and progress.student_id = $2
        "#,
        assignment_ids,
        student_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?;

    rows.into_iter()
        .map(|row| {
            let history = SubmissionHistory::new(
                checked_num_cast("submission_history.used_attempts", row.used_attempts)?,
                checked_num_cast("submission_history.extra_attempts", row.extra_attempts)?,
                row.has_accepted_submission,
                row.has_pending_submission,
            );

            Ok((AssignmentId::from_uuid(row.assignment_id), history))
        })
        .collect()
}

pub async fn list_for_student<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    student_id: UserId,
    page: PageRequest,
) -> DbResult<Vec<Submission>>
where
    E: PgExecutor<'e>,
{
    select_submissions!(
        "where assignment_id = $1 and student_id = $2 order by attempt_number desc limit $3 offset $4",
        assignment_id.as_uuid(),
        student_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(Submission::try_from)
    .collect()
}

pub async fn latest_for_student<'e, E: PgExecutor<'e>>(
    executor: E,
    assignment_id: AssignmentId,
    student_id: UserId,
) -> DbResult<Option<Submission>> {
    select_submissions!(
        "where assignment_id = $1 and student_id = $2 order by attempt_number desc limit 1",
        assignment_id.as_uuid(),
        student_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?
    .map(Submission::try_from)
    .transpose()
}

#[derive(Clone, Debug)]
pub struct CourseGradeRow {
    pub student_id: UserId,
    pub display_name: String,
    pub assignment: Option<AssignmentGrade>,
    pub total: u32,
    pub max_total: u32,
}

/// Сводка одним стейтментом: студенты группы слева, оценочные задания курса и принятые сдачи
/// справа. left joins обязательны - студент без единой сдачи должен вернуться нулевой
/// строкой, а не пропасть из ведомости.
pub async fn course_grades<'e, E>(
    executor: E,
    teaching_course_id: TeachingCourseId,
    student_group_id: StudentGroupId,
    student_page: PageRequest,
    student_id: Option<UserId>,
    grade_page: PageRequest,
) -> DbResult<Vec<CourseGradeRow>>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        with students_page as (
            select id, display_name from users
            where role = 'student' and student_group_id = $2
              and ($5::uuid is null or id = $5)
            order by display_name, id limit $3 offset $4
        )
        select
            students.id as "student_id!",
            students.display_name as "display_name!",
            grades.id as "assignment_id?",
            grades.title as "assignment_title?",
            grades.max_score as "max_score?",
            grades.final_score as "final_score?",
            coalesce(totals.total, 0)::bigint as "total!",
            coalesce(totals.max_total, 0)::bigint as "max_total!"
        from students_page students
        cross join lateral (
            select sum(s.final_score) as total,
                   sum(coalesce(s.grade_max_score, a.max_score)) as max_total
            from assignments a
            left join submissions s on s.assignment_id = a.id
                and s.student_id = students.id and s.status = 'accepted'
            where a.teaching_course_id = $1 and a.published_at is not null
        ) totals
        left join lateral (
            select a.id, a.title, a.deadline_at,
                   coalesce(s.grade_max_score, a.max_score) as max_score, s.final_score
            from assignments a
            left join submissions s on s.assignment_id = a.id
                and s.student_id = students.id and s.status = 'accepted'
            where a.teaching_course_id = $1 and a.published_at is not null
              and coalesce(s.grade_max_score, a.max_score) is not null
            order by a.deadline_at, a.id limit $6 offset $7
        ) grades on true
        order by students.display_name, students.id, grades.deadline_at, grades.id
        "#,
        teaching_course_id.as_uuid(),
        student_group_id.as_uuid(),
        student_page.fetch_limit(),
        student_page.offset(),
        student_id.map(UserId::as_uuid),
        grade_page.fetch_limit(),
        grade_page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let assignment = match (row.assignment_id, row.assignment_title, row.max_score) {
            (Some(id), Some(title), Some(max_score)) => Some(AssignmentGrade {
                assignment_id: AssignmentId::from_uuid(id),
                title,
                max_score: checked_num_cast(MAX_SCORE, max_score)?,
                final_score: cast_score(FINAL_SCORE, row.final_score)?,
            }),
            _ => None,
        };

        Ok(CourseGradeRow {
            student_id: UserId::from_uuid(row.student_id),
            display_name: row.display_name,
            assignment,
            total: checked_num_cast("gradebook.total", row.total)?,
            max_total: checked_num_cast("gradebook.max_total", row.max_total)?,
        })
    })
    .collect()
}

/// Место сдачи студента в очереди проверки. Очередь ревьюера склеена из всех его курсов, поэтому
/// считаем не по курсу задания, а по всем сдачам, которые видит хотя бы один его преподаватель.
/// Порядок обязан совпадать с `lock_next_pending_for_reviewer`.
pub async fn pending_queue_places<'e, E>(
    executor: E,
    student_id: UserId,
    page: PageRequest,
    assignment_id: Option<AssignmentId>,
) -> DbResult<Vec<QueuePlace>>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        with mine_page as (
            select * from submissions where student_id = $1 and status = 'pending_review'
              and ($4::uuid is null or assignment_id = $4)
            order by submitted_at, id limit $2 offset $3
        ), visible as (
            select
                mine.id as mine_id,
                other.id as other_id,
                other.submitted_at as other_submitted_at
            from mine_page mine
            join assignments mine_assignment
              on mine_assignment.id = mine.assignment_id
            join assignments other_assignment
              on exists (
                  select 1
                  from teaching_course_teachers mine_teacher
                  join teaching_course_teachers other_teacher
                    on other_teacher.teacher_id = mine_teacher.teacher_id
                  where mine_teacher.teaching_course_id = mine_assignment.teaching_course_id
                    and other_teacher.teaching_course_id = other_assignment.teaching_course_id
              )
            join submissions other
              on other.assignment_id = other_assignment.id
             and other.status = 'pending_review'
            where mine.student_id = $1
              and mine.status = 'pending_review'
        )
        select
            submissions.assignment_id as "assignment_id!",
            subjects.name as "subject_name!",
            assignments.title as "assignment_title!",
            submissions.submitted_at as "submitted_at!",
            count(visible.other_id) filter (
                where (visible.other_submitted_at, visible.other_id)
                   <= (submissions.submitted_at, submissions.id)
            ) as "position!",
            count(visible.other_id) as "total!"
        from mine_page submissions
        join assignments
          on assignments.id = submissions.assignment_id
        join subjects
          on subjects.id = assignments.subject_id
        left join visible
          on visible.mine_id = submissions.id
        where submissions.student_id = $1
          and submissions.status = 'pending_review'
        group by submissions.id, submissions.submitted_at, submissions.assignment_id,
                 subjects.name, assignments.title
        order by submissions.submitted_at, submissions.id
        "#,
        student_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
        assignment_id.map(AssignmentId::as_uuid),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        Ok(QueuePlace {
            assignment_id: AssignmentId::from_uuid(row.assignment_id),
            subject_name: row.subject_name,
            assignment_title: row.assignment_title,
            submitted_at: row.submitted_at,
            position: checked_num_cast("review_queue.position", row.position)?,
            total: checked_num_cast("review_queue.total", row.total)?,
        })
    })
    .collect()
}

/// Видимость обязана совпадать с `lock_next_pending_for_reviewer`!!
/// иначе бот может показать например очередь из трёх сдач, а `POST /submissions/next` вернёт 204.
pub async fn count_pending_for_reviewer<'e, E>(executor: E, reviewer: &User) -> DbResult<u32>
where
    E: PgExecutor<'e>,
{
    let can_review_all = matches!(reviewer.role(), Role::Admin);
    let pending = sqlx::query_scalar!(
        r#"
        select count(*) as "pending!"
        from submissions
        join assignments
          on assignments.id = submissions.assignment_id
        left join teaching_course_teachers
          on teaching_course_teachers.teaching_course_id = assignments.teaching_course_id
         and teaching_course_teachers.teacher_id = $2
        where submissions.status = 'pending_review'
          and ($1 or teaching_course_teachers.teacher_id is not null)
        "#,
        can_review_all,
        reviewer.id().as_uuid(),
    )
    .fetch_one(executor)
    .await?;

    checked_num_cast("review_queue.pending", pending)
}

pub async fn lock_next_pending_for_reviewer(
    tx: &mut Transaction<'_, Postgres>,
    reviewer: &User,
) -> DbResult<Option<Submission>> {
    let can_review_all = matches!(reviewer.role(), Role::Admin);
    let row = sqlx::query_as!(
        SubmissionRow,
        r#"
        select
            submissions.id, submissions.assignment_id, submissions.student_id,
            submissions.attempt_number, submissions.content, submissions.status,
            submissions.submitted_at, submissions.reviewed_by, submissions.reviewed_at,
            submissions.review_comment, submissions.raw_score, submissions.grade_max_score,
            submissions.late_periods, submissions.penalty, submissions.final_score
        from submissions
        join assignments
          on assignments.id = submissions.assignment_id
        left join teaching_course_teachers
          on teaching_course_teachers.teaching_course_id = assignments.teaching_course_id
         and teaching_course_teachers.teacher_id = $2
        where submissions.status = 'pending_review'
          and ($1 or teaching_course_teachers.teacher_id is not null)
        order by submissions.submitted_at, submissions.id
        for update of submissions skip locked
        limit 1
        "#,
        can_review_all,
        reviewer.id().as_uuid(),
    )
    .fetch_optional(&mut **tx)
    .await?;

    row.map(Submission::try_from).transpose()
}

#[derive(Debug)]
struct SubmissionRow {
    id: Uuid,
    assignment_id: Uuid,
    student_id: Uuid,
    attempt_number: i16,
    content: String,
    status: String,
    submitted_at: DateTime<Utc>,
    reviewed_by: Option<Uuid>,
    reviewed_at: Option<DateTime<Utc>>,
    review_comment: Option<String>,
    raw_score: Option<i16>,
    grade_max_score: Option<i16>,
    late_periods: Option<i32>,
    penalty: Option<i16>,
    final_score: Option<i16>,
}

fn cast_score(column: &'static str, value: Option<i16>) -> DbResult<Option<u16>> {
    value
        .map(|value| checked_num_cast(column, value))
        .transpose()
}

impl TryFrom<SubmissionRow> for Submission {
    type Error = DbError;

    fn try_from(row: SubmissionRow) -> Result<Self, Self::Error> {
        let status = SubmissionStatus::parse_str(&row.status)
            .ok_or_else(|| DbError::invalid_value(STATUS, &row.status))?;
        let attempt_number: u16 = checked_num_cast(ATTEMPT_NUMBER, row.attempt_number)?;

        Ok(Submission::restore(SubmissionParts {
            id: SubmissionId::from_uuid(row.id),
            assignment_id: AssignmentId::from_uuid(row.assignment_id),
            student_id: UserId::from_uuid(row.student_id),
            attempt_number,
            content: row.content,
            status,
            submitted_at: row.submitted_at,
            reviewed_by: row.reviewed_by.map(UserId::from_uuid),
            reviewed_at: row.reviewed_at,
            review_comment: row.review_comment,
            raw_score: cast_score(RAW_SCORE, row.raw_score)?,
            grade_max_score: cast_score(GRADE_MAX_SCORE, row.grade_max_score)?,
            late_periods: row
                .late_periods
                .map(|value| checked_num_cast(LATE_PERIODS, value))
                .transpose()?,
            penalty: cast_score(PENALTY, row.penalty)?,
            final_score: cast_score(FINAL_SCORE, row.final_score)?,
        })?)
    }
}
