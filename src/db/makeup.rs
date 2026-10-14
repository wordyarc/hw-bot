use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::db::ensure_updated;
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, CourseMakeup, Grading, MakeupPool, MaxScore, TeachingCourseId, UserId,
};
use crate::read_models::UndecidedAssignment;

const MAKEUP: &str = "teaching_courses.makeup";
const MAKEUP_ATTEMPTS: &str = "teaching_courses.makeup_attempts";
const EXTRA_MAKEUP_ATTEMPTS: &str = "student_course_progress.extra_makeup_attempts";
const SPENT_MAKEUP_ATTEMPTS: &str = "student_course_progress.spent";
const MAX_SCORE: &str = "assignments.max_score";
const MAKEUP_CREDIT: &str = "assignments.makeup_credit";

struct CourseMakeupColumns {
    until: Option<DateTime<Utc>>,
    attempts: Option<i16>,
    opened_at: Option<DateTime<Utc>>,
}

impl CourseMakeupColumns {
    fn of(makeup: Option<CourseMakeup>) -> DbResult<Self> {
        Ok(Self {
            until: makeup.map(CourseMakeup::until),
            attempts: makeup
                .map(|makeup| checked_num_cast(MAKEUP_ATTEMPTS, makeup.attempts()))
                .transpose()?,
            opened_at: makeup.map(CourseMakeup::opened_at),
        })
    }

    fn restore(self) -> DbResult<Option<CourseMakeup>> {
        match (self.until, self.attempts, self.opened_at) {
            (None, None, None) => Ok(None),
            (Some(until), Some(attempts), Some(opened_at)) => Ok(Some(CourseMakeup::restore(
                until,
                checked_num_cast(MAKEUP_ATTEMPTS, attempts)?,
                opened_at,
            )?)),
            _ => Err(DbError::invalid_value(MAKEUP, "incomplete settings")),
        }
    }
}

pub async fn find_course(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
) -> DbResult<Option<CourseMakeup>> {
    let Some(row) = sqlx::query!(
        r#"
        select makeup_until, makeup_attempts, makeup_opened_at
        from teaching_courses
        where id = $1
        "#,
        course_id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?
    else {
        return Ok(None);
    };

    CourseMakeupColumns {
        until: row.makeup_until,
        attempts: row.makeup_attempts,
        opened_at: row.makeup_opened_at,
    }
    .restore()
}

pub async fn update_course(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    expected: Option<CourseMakeup>,
    makeup: Option<CourseMakeup>,
) -> DbResult<Option<CourseMakeup>> {
    let expected = CourseMakeupColumns::of(expected)?;
    let makeup = CourseMakeupColumns::of(makeup)?;

    let row = sqlx::query!(
        r#"
        update teaching_courses
        set
            makeup_until = $2,
            makeup_attempts = $3,
            makeup_opened_at = $4
        where id = $1
          and makeup_until is not distinct from $5
          and makeup_attempts is not distinct from $6
          and makeup_opened_at is not distinct from $7
        returning makeup_until, makeup_attempts, makeup_opened_at
        "#,
        course_id.as_uuid(),
        makeup.until,
        makeup.attempts,
        makeup.opened_at,
        expected.until,
        expected.attempts,
        expected.opened_at,
    )
    .fetch_optional(executor)
    .await?
    .ok_or(DbError::Conflict {
        message: "course makeup settings were changed by a concurrent request",
    })?;

    CourseMakeupColumns {
        until: row.makeup_until,
        attempts: row.makeup_attempts,
        opened_at: row.makeup_opened_at,
    }
    .restore()
}

pub async fn clear_pool(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
) -> DbResult<()> {
    sqlx::query!(
        r#"
        delete from student_course_progress
        where teaching_course_id = $1
        "#,
        course_id.as_uuid(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn lock_pool(
    tx: &mut Transaction<'_, Postgres>,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> DbResult<u16> {
    let extra = sqlx::query_scalar!(
        r#"
        insert into student_course_progress (teaching_course_id, student_id, extra_makeup_attempts)
        values ($1, $2, 0)
        on conflict (teaching_course_id, student_id) do update
        set extra_makeup_attempts = student_course_progress.extra_makeup_attempts
        returning extra_makeup_attempts
        "#,
        course_id.as_uuid(),
        student_id.as_uuid(),
    )
    .fetch_one(&mut **tx)
    .await?;

    checked_num_cast(EXTRA_MAKEUP_ATTEMPTS, extra)
}

pub async fn set_extra_attempts(
    tx: &mut Transaction<'_, Postgres>,
    course_id: TeachingCourseId,
    student_id: UserId,
    extra: u16,
) -> DbResult<()> {
    let result = sqlx::query!(
        r#"
        update student_course_progress
        set extra_makeup_attempts = $3
        where teaching_course_id = $1
          and student_id = $2
        "#,
        course_id.as_uuid(),
        student_id.as_uuid(),
        checked_num_cast::<_, i16>(EXTRA_MAKEUP_ATTEMPTS, extra)?,
    )
    .execute(&mut **tx)
    .await?;

    ensure_updated(
        result,
        DbError::Conflict {
            message: "student course progress was removed by a concurrent request",
        },
    )
}

pub async fn pool_for_student(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    student_id: UserId,
) -> DbResult<Option<MakeupPool>> {
    Ok(
        pools_for_student(executor, &[course_id.as_uuid()], student_id)
            .await?
            .remove(&course_id),
    )
}

pub async fn pools_for_student(
    executor: impl PgExecutor<'_>,
    course_ids: &[Uuid],
    student_id: UserId,
) -> DbResult<HashMap<TeachingCourseId, MakeupPool>> {
    let rows = sqlx::query!(
        r#"
        select
            courses.id,
            courses.makeup_until,
            courses.makeup_attempts,
            courses.makeup_opened_at,
            coalesce(progress.extra_makeup_attempts, 0)::smallint as "extra!",
            (
                select count(*)
                from submissions
                join assignments
                  on assignments.id = submissions.assignment_id
                where assignments.teaching_course_id = courses.id
                  and submissions.student_id = $2
                  and submissions.kind = 'makeup'
                  and submissions.submitted_at >= courses.makeup_opened_at
            ) as "spent!"
        from teaching_courses courses
        left join student_course_progress progress
          on progress.teaching_course_id = courses.id
         and progress.student_id = $2
        where courses.id = any($1)
          and courses.makeup_opened_at is not null
        "#,
        course_ids,
        student_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?;

    let mut pools = HashMap::with_capacity(rows.len());
    for row in rows {
        let columns = CourseMakeupColumns {
            until: row.makeup_until,
            attempts: row.makeup_attempts,
            opened_at: row.makeup_opened_at,
        };
        let Some(course) = columns.restore()? else {
            continue;
        };
        let balance = course.balance(
            checked_num_cast(EXTRA_MAKEUP_ATTEMPTS, row.extra)?,
            checked_num_cast(SPENT_MAKEUP_ATTEMPTS, row.spent)?,
        );
        pools.insert(
            TeachingCourseId::from_uuid(row.id),
            MakeupPool { course, balance },
        );
    }

    Ok(pools)
}

pub async fn undecided_assignments(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
) -> DbResult<Vec<UndecidedAssignment>> {
    sqlx::query!(
        r#"
        select id, title, max_score
        from assignments
        where teaching_course_id = $1
          and status <> 'archived'
          and makeup_offered is null
        order by max_score desc nulls last, created_at, id
        "#,
        course_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let max_score = row
            .max_score
            .map(|value| checked_num_cast::<_, u16>(MAX_SCORE, value))
            .transpose()?;

        Ok(UndecidedAssignment {
            assignment_id: AssignmentId::from_uuid(row.id),
            title: row.title,
            grading: Grading::from_max_score(max_score)?,
        })
    })
    .collect()
}

pub async fn common_credit(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    max_score: MaxScore,
    except: Option<AssignmentId>,
) -> DbResult<Option<u16>> {
    let row = sqlx::query!(
        r#"
        select min(makeup_credit) as lowest, max(makeup_credit) as highest
        from assignments
        where teaching_course_id = $1
          and max_score = $2
          and makeup_credit is not null
          and status <> 'archived'
          and ($3::uuid is null or id <> $3)
        "#,
        course_id.as_uuid(),
        checked_num_cast::<_, i16>(MAX_SCORE, max_score.get())?,
        except.map(AssignmentId::as_uuid),
    )
    .fetch_one(executor)
    .await?;

    match (row.lowest, row.highest) {
        (Some(lowest), Some(highest)) if lowest == highest => {
            Ok(Some(checked_num_cast(MAKEUP_CREDIT, lowest)?))
        }
        _ => Ok(None),
    }
}
