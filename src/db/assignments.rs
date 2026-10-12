use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::ensure_updated;
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    Assignment, AssignmentId, AssignmentParts, AssignmentStatus, DeductedFrom, Grading, LatePolicy,
    MakeupTerms, StudentGroupId, Subject, SubjectId, TeachingCourseId, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::VersionedAssignment;

const MAX_ATTEMPTS: &str = "assignments.max_attempts";
const MAX_SCORE: &str = "assignments.max_score";
const LATE_PERIOD_DAYS: &str = "assignments.late_period_days";
const LATE_PENALTY_PER_PERIOD: &str = "assignments.late_penalty_per_period";
const LATE_MAX_PERIODS: &str = "assignments.late_max_periods";
const LATE_DEDUCTED_FROM: &str = "assignments.late_deducted_from";
const MAKEUP_CREDIT: &str = "assignments.makeup_credit";
const STATUS: &str = "assignments.status";

fn max_score_param(assignment: &Assignment) -> DbResult<Option<i16>> {
    assignment
        .grading()
        .max_score()
        .map(|value| checked_num_cast::<_, i16>(MAX_SCORE, value))
        .transpose()
}

struct LatePolicyColumns {
    period_days: Option<i16>,
    penalty_per_period: Option<i16>,
    max_periods: Option<i16>,
    deducted_from: Option<&'static str>,
}

impl LatePolicyColumns {
    fn of(assignment: &Assignment) -> DbResult<Self> {
        let Some(policy) = assignment.late_policy() else {
            return Ok(Self {
                period_days: None,
                penalty_per_period: None,
                max_periods: None,
                deducted_from: None,
            });
        };

        Ok(Self {
            period_days: Some(checked_num_cast(LATE_PERIOD_DAYS, policy.period_days())?),
            penalty_per_period: Some(checked_num_cast(
                LATE_PENALTY_PER_PERIOD,
                policy.penalty_per_period(),
            )?),
            max_periods: Some(checked_num_cast(LATE_MAX_PERIODS, policy.max_periods())?),
            deducted_from: Some(policy.deducted_from().as_str()),
        })
    }
}

struct MakeupColumns {
    offered: Option<bool>,
    credit: Option<i16>,
}

impl MakeupColumns {
    fn of(assignment: &Assignment) -> DbResult<Self> {
        let terms = assignment.makeup();

        Ok(Self {
            offered: terms.map(MakeupTerms::is_offered),
            credit: terms
                .and_then(MakeupTerms::credit)
                .map(|credit| checked_num_cast(MAKEUP_CREDIT, credit.get()))
                .transpose()?,
        })
    }
}

fn late_policy_from_row(
    period_days: Option<i16>,
    penalty_per_period: Option<i16>,
    max_periods: Option<i16>,
    deducted_from: Option<String>,
) -> DbResult<Option<LatePolicy>> {
    let (Some(period_days), Some(penalty_per_period), Some(max_periods), Some(deducted_from)) =
        (period_days, penalty_per_period, max_periods, deducted_from)
    else {
        return Ok(None);
    };
    let deducted_from = DeductedFrom::parse_str(&deducted_from)
        .ok_or_else(|| DbError::invalid_value(LATE_DEDUCTED_FROM, &deducted_from))?;

    Ok(Some(LatePolicy::new(
        checked_num_cast(LATE_PERIOD_DAYS, period_days)?,
        checked_num_cast(LATE_PENALTY_PER_PERIOD, penalty_per_period)?,
        checked_num_cast(LATE_MAX_PERIODS, max_periods)?,
        deducted_from,
    )?))
}

macro_rules! assignment_row {
    ($row:expr) => {
        AssignmentRow {
            id: $row.id,
            teaching_course_id: $row.teaching_course_id,
            subject_id: $row.subject_id,
            group_ids: $row.group_ids,
            title: $row.title,
            body: $row.body,
            max_attempts: $row.max_attempts,
            max_score: $row.max_score,
            late_period_days: $row.late_period_days,
            late_penalty_per_period: $row.late_penalty_per_period,
            late_max_periods: $row.late_max_periods,
            late_deducted_from: $row.late_deducted_from,
            makeup_offered: $row.makeup_offered,
            makeup_credit: $row.makeup_credit,
            status: $row.status,
            created_by: $row.created_by,
            created_at: $row.created_at,
            deadline_at: $row.deadline_at,
            published_at: $row.published_at,
        }
    };
}

macro_rules! select_assignments {
    ($extra:literal, $tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query!(
            r#"
            select
                id, teaching_course_id, subject_id,
                array(
                    select student_group_id
                    from assignment_groups
                    where assignment_id = assignments.id
                ) as "group_ids!: Vec<Uuid>",
                title, body, max_attempts, max_score, late_period_days, late_penalty_per_period,
                late_max_periods, late_deducted_from, makeup_offered, makeup_credit, status,
                created_by, created_at, deadline_at, published_at
            "# + $extra + $tail
            $(, $arg)*
        )
    };
}

macro_rules! fetch_assignment_cards {
    ($executor:expr, $tail:literal $(, $arg:expr)* $(,)?) => {
        sqlx::query!(
            r#"
            select
                assignments.id, assignments.teaching_course_id, assignments.subject_id,
                array(
                    select student_group_id
                    from assignment_groups
                    where assignment_id = assignments.id
                ) as "group_ids!: Vec<Uuid>",
                assignments.title, assignments.body,
                assignments.max_attempts, assignments.max_score, assignments.late_period_days,
                assignments.late_penalty_per_period, assignments.late_max_periods,
                assignments.late_deducted_from, assignments.makeup_offered,
                assignments.makeup_credit, assignments.status, assignments.created_by,
                assignments.created_at, assignments.deadline_at, assignments.published_at,
                subjects.name as subject_name
            from assignments
            join subjects
              on subjects.id = assignments.subject_id
            "# + $tail
            $(, $arg)*
        )
        .fetch_all($executor)
        .await?
        .into_iter()
        .map(|row| {
            Ok(StudentAssignmentCard {
                subject: Subject::new(SubjectId::from_uuid(row.subject_id), row.subject_name)?,
                assignment: Assignment::try_from(assignment_row!(row))?,
            })
        })
        .collect()
    };
}

pub async fn insert(conn: &mut PgConnection, assignment: &Assignment) -> DbResult<()> {
    let late_policy = LatePolicyColumns::of(assignment)?;
    let makeup = MakeupColumns::of(assignment)?;

    sqlx::query!(
        r#"
        insert into assignments (
            id, teaching_course_id, subject_id, title, body, max_attempts, max_score,
            late_period_days, late_penalty_per_period, late_max_periods, late_deducted_from,
            makeup_offered, makeup_credit, status, created_by, created_at, deadline_at,
            published_at
        )
        values (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18
        )
        "#,
        assignment.id().as_uuid(),
        assignment.teaching_course_id().as_uuid(),
        assignment.subject_id().as_uuid(),
        assignment.title().as_str(),
        assignment.body().as_str(),
        checked_num_cast::<_, i16>(MAX_ATTEMPTS, assignment.max_attempts())?,
        max_score_param(assignment)?,
        late_policy.period_days,
        late_policy.penalty_per_period,
        late_policy.max_periods,
        late_policy.deducted_from,
        makeup.offered,
        makeup.credit,
        assignment.status().as_str(),
        assignment.created_by().as_uuid(),
        assignment.created_at(),
        assignment.deadline_at(),
        assignment.published_at(),
    )
    .execute(&mut *conn)
    .await?;

    let group_ids: Vec<Uuid> = assignment.groups().iter().map(|id| id.as_uuid()).collect();

    sqlx::query!(
        r#"
        insert into assignment_groups (assignment_id, teaching_course_id, student_group_id)
        select $1, $2, student_group_id
        from unnest($3::uuid[]) as issued (student_group_id)
        "#,
        assignment.id().as_uuid(),
        assignment.teaching_course_id().as_uuid(),
        &group_ids,
    )
    .execute(conn)
    .await?;

    Ok(())
}

pub async fn update<'e, E>(
    executor: E,
    assignment: &Assignment,
    expected_revision: i64,
) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    let late_policy = LatePolicyColumns::of(assignment)?;
    let makeup = MakeupColumns::of(assignment)?;

    let result = sqlx::query!(
        r#"
        update assignments
        set
            status = $2,
            deadline_at = $3,
            published_at = $4,
            max_score = $5,
            late_period_days = $6,
            late_penalty_per_period = $7,
            late_max_periods = $8,
            late_deducted_from = $9,
            makeup_offered = $10,
            makeup_credit = $11,
            revision = revision + 1
        where id = $1
          and revision = $12
        "#,
        assignment.id().as_uuid(),
        assignment.status().as_str(),
        assignment.deadline_at(),
        assignment.published_at(),
        max_score_param(assignment)?,
        late_policy.period_days,
        late_policy.penalty_per_period,
        late_policy.max_periods,
        late_policy.deducted_from,
        makeup.offered,
        makeup.credit,
        expected_revision,
    )
    .execute(executor)
    .await?;

    ensure_updated(
        result,
        DbError::Conflict {
            message: "assignment was changed by a concurrent request",
        },
    )
}

pub async fn advance_revision<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    expected_revision: i64,
) -> DbResult<i64>
where
    E: PgExecutor<'e>,
{
    sqlx::query_scalar!(
        r#"
        update assignments
        set revision = revision + 1
        where id = $1
          and revision = $2
        returning revision
        "#,
        assignment_id.as_uuid(),
        expected_revision,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::Conflict {
        message: "assignment was changed by a concurrent request",
    })
}

pub async fn find_versioned_by_id<'e, E>(
    executor: E,
    id: AssignmentId,
) -> DbResult<Option<VersionedAssignment>>
where
    E: PgExecutor<'e>,
{
    let row = select_assignments!(
        ", revision ",
        " from assignments where id = $1",
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(|row| {
        Ok(VersionedAssignment {
            assignment: Assignment::try_from(assignment_row!(row))?,
            revision: row.revision,
        })
    })
    .transpose()
}

pub async fn find_by_id<'e, E>(executor: E, id: AssignmentId) -> DbResult<Option<Assignment>>
where
    E: PgExecutor<'e>,
{
    let row = select_assignments!("", " from assignments where id = $1", id.as_uuid(),)
        .fetch_optional(executor)
        .await?;

    row.map(|row| Assignment::try_from(assignment_row!(row)))
        .transpose()
}

pub async fn list_published_for_group<'e, E>(
    executor: E,
    student_group_id: StudentGroupId,
    page: PageRequest,
) -> DbResult<Vec<StudentAssignmentCard>>
where
    E: PgExecutor<'e>,
{
    fetch_assignment_cards!(
        executor,
        " join assignment_groups issued
            on issued.assignment_id = assignments.id
         where issued.student_group_id = $1
           and assignments.status = 'published'
         order by subjects.name, assignments.deadline_at, assignments.id limit $2 offset $3",
        student_group_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
    )
}

pub async fn list_archived_for_group<'e, E>(
    executor: E,
    student_group_id: StudentGroupId,
    page: PageRequest,
) -> DbResult<Vec<StudentAssignmentCard>>
where
    E: PgExecutor<'e>,
{
    fetch_assignment_cards!(
        executor,
        " join assignment_groups issued
            on issued.assignment_id = assignments.id
         where issued.student_group_id = $1
           and assignments.status = 'archived'
           and assignments.published_at is not null
         order by subjects.name, assignments.published_at desc, assignments.id limit $2 offset $3",
        student_group_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
    )
}

pub async fn list_for_course<'e, E>(
    executor: E,
    teaching_course_id: TeachingCourseId,
    page: PageRequest,
) -> DbResult<Vec<Assignment>>
where
    E: PgExecutor<'e>,
{
    select_assignments!(
        "",
        " from assignments where teaching_course_id = $1 order by created_at desc, id limit $2 offset $3",
        teaching_course_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| Assignment::try_from(assignment_row!(row)))
    .collect()
}

#[derive(Debug)]
struct AssignmentRow {
    id: Uuid,
    teaching_course_id: Uuid,
    subject_id: Uuid,
    group_ids: Vec<Uuid>,
    title: String,
    body: String,
    max_attempts: i16,
    max_score: Option<i16>,
    late_period_days: Option<i16>,
    late_penalty_per_period: Option<i16>,
    late_max_periods: Option<i16>,
    late_deducted_from: Option<String>,
    makeup_offered: Option<bool>,
    makeup_credit: Option<i16>,
    status: String,
    created_by: Uuid,
    created_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct StudentAssignmentCard {
    pub assignment: Assignment,
    pub subject: Subject,
}

impl TryFrom<AssignmentRow> for Assignment {
    type Error = DbError;

    fn try_from(row: AssignmentRow) -> Result<Self, Self::Error> {
        let max_attempts: u16 = checked_num_cast(MAX_ATTEMPTS, row.max_attempts)?;
        let max_score = row
            .max_score
            .map(|value| checked_num_cast::<_, u16>(MAX_SCORE, value))
            .transpose()?;
        let status = AssignmentStatus::parse_str(&row.status)
            .ok_or_else(|| DbError::invalid_value(STATUS, &row.status))?;
        let grading = Grading::from_max_score(max_score)?;
        let makeup_credit = row
            .makeup_credit
            .map(|value| checked_num_cast::<_, u16>(MAKEUP_CREDIT, value))
            .transpose()?;

        Ok(Assignment::restore(AssignmentParts {
            id: AssignmentId::from_uuid(row.id),
            teaching_course_id: TeachingCourseId::from_uuid(row.teaching_course_id),
            subject_id: SubjectId::from_uuid(row.subject_id),
            groups: row
                .group_ids
                .into_iter()
                .map(StudentGroupId::from_uuid)
                .collect(),
            title: row.title,
            body: row.body,
            max_attempts,
            grading,
            late_policy: late_policy_from_row(
                row.late_period_days,
                row.late_penalty_per_period,
                row.late_max_periods,
                row.late_deducted_from,
            )?,
            makeup: MakeupTerms::restore(row.makeup_offered, makeup_credit, grading)?,
            status,
            created_by: UserId::from_uuid(row.created_by),
            created_at: row.created_at,
            deadline_at: row.deadline_at,
            published_at: row.published_at,
        })?)
    }
}
