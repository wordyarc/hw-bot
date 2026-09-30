use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, Role, StudentGroupId, SubmissionHistory, TelegramUserId, User, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::{AssignmentStudentSummary, CourseStudent, UserStatistics};

pub async fn insert<'e, E>(executor: E, user: &User) -> DbResult<()>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        insert into users (
            id, telegram_user_id, role, display_name, student_group_id, registered_at
        )
        values ($1, $2, $3, $4, $5, $6)
        "#,
        user.id().as_uuid(),
        user.telegram_user_id().as_i64(),
        user.role().as_str(),
        user.display_name().as_str(),
        user.student_group_id().map(StudentGroupId::as_uuid),
        user.registered_at(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn find_by_id<'e, E>(executor: E, id: UserId) -> DbResult<Option<User>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, telegram_user_id, role, display_name, student_group_id, registered_at
        from users
        where id = $1
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(User::try_from).transpose()
}

pub async fn find_by_telegram_user_id<'e, E>(
    executor: E,
    telegram_user_id: TelegramUserId,
) -> DbResult<Option<User>>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, telegram_user_id, role, display_name, student_group_id, registered_at
        from users
        where telegram_user_id = $1
        "#,
        telegram_user_id.as_i64(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(User::try_from).transpose()
}

pub(crate) async fn list_students_with_assignment_history<'e, E>(
    executor: E,
    assignment_id: AssignmentId,
    student_group_id: StudentGroupId,
    page: PageRequest,
) -> DbResult<Vec<AssignmentStudentSummary>>
where
    E: PgExecutor<'e>,
{
    sqlx::query_as!(
        AssignmentStudentCardRow,
        r#"
        with history as (
            select
                student_id,
                count(*) filter (
                    where status in ('accepted', 'rejected')
                )::bigint as used_attempts,
                bool_or(status = 'accepted') as has_accepted_submission,
                bool_or(status = 'pending_review') as has_pending_submission
            from submissions
            where assignment_id = $1
            group by student_id
        )
        select
            users.id,
            users.telegram_user_id,
            users.role,
            users.display_name,
            users.student_group_id,
            users.registered_at,
            coalesce(history.used_attempts, 0)::bigint as "used_attempts!",
            coalesce(progress.extra_attempts, 0)::smallint as "extra_attempts!",
            coalesce(history.has_accepted_submission, false) as "has_accepted_submission!",
            coalesce(history.has_pending_submission, false) as "has_pending_submission!"
        from users
        left join history
          on history.student_id = users.id
        left join student_assignment_progress progress
          on progress.assignment_id = $1
         and progress.student_id = users.id
        where users.role = 'student'
          and users.student_group_id = $2
        order by users.display_name, users.id
        limit $3 offset $4
        "#,
        assignment_id.as_uuid(),
        student_group_id.as_uuid(),
        PageRequest::FETCH_LIMIT,
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(AssignmentStudentSummary::try_from)
    .collect()
}

pub async fn profile_statistics<'e, E>(executor: E, user: &User) -> DbResult<UserStatistics>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query!(
        r#"
        select
            (
                select count(*)
                from assignments
                where student_group_id = $2
                  and status = 'published'
            ) as "assignments!",
            (
                select count(*)
                from submissions
                where student_id = $1
                  and status = 'pending_review'
            ) as "pending!",
            (
                select count(*)
                from teaching_courses course
                where $3 or exists (
                    select 1
                    from teaching_course_teachers teacher
                    where teacher.teaching_course_id = course.id
                      and teacher.teacher_id = $1
                )
            ) as "courses!"
        "#,
        user.id().as_uuid(),
        user.student_group_id().map(StudentGroupId::as_uuid),
        user.role().can_manage_all_teaching_courses(),
    )
    .fetch_one(executor)
    .await?;

    Ok(UserStatistics {
        assignments: checked_num_cast("profile.assignments", row.assignments)?,
        pending: checked_num_cast("profile.pending", row.pending)?,
        courses: checked_num_cast("profile.courses", row.courses)?,
    })
}

pub async fn list_student_names<'e, E>(
    executor: E,
    group_id: StudentGroupId,
    page: PageRequest,
) -> DbResult<Vec<CourseStudent>>
where
    E: PgExecutor<'e>,
{
    let rows = sqlx::query!(
        r#"
        select id, display_name
        from users
        where student_group_id = $1
          and role = 'student'
        order by display_name, id
        limit $2 offset $3
        "#,
        group_id.as_uuid(),
        PageRequest::FETCH_LIMIT,
        page.offset(),
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| CourseStudent {
            student_id: UserId::from_uuid(row.id),
            display_name: row.display_name,
        })
        .collect())
}

#[derive(Debug)]
struct UserRow {
    id: Uuid,
    telegram_user_id: i64,
    role: String,
    display_name: String,
    student_group_id: Option<Uuid>,
    registered_at: DateTime<Utc>,
}

#[derive(Debug)]
struct AssignmentStudentCardRow {
    id: Uuid,
    telegram_user_id: i64,
    role: String,
    display_name: String,
    student_group_id: Option<Uuid>,
    registered_at: DateTime<Utc>,
    used_attempts: i64,
    extra_attempts: i16,
    has_accepted_submission: bool,
    has_pending_submission: bool,
}

impl TryFrom<UserRow> for User {
    type Error = DbError;

    fn try_from(row: UserRow) -> Result<Self, Self::Error> {
        let telegram_user_id = TelegramUserId::new(row.telegram_user_id)?;
        let role = Role::from_db_str(&row.role, row.student_group_id)?;

        Ok(User::new(
            UserId::from_uuid(row.id),
            telegram_user_id,
            role,
            row.display_name,
            row.registered_at,
        )?)
    }
}

impl TryFrom<AssignmentStudentCardRow> for AssignmentStudentSummary {
    type Error = DbError;

    fn try_from(row: AssignmentStudentCardRow) -> Result<Self, Self::Error> {
        let student = User::try_from(UserRow {
            id: row.id,
            telegram_user_id: row.telegram_user_id,
            role: row.role,
            display_name: row.display_name,
            student_group_id: row.student_group_id,
            registered_at: row.registered_at,
        })?;
        let history = SubmissionHistory::new(
            checked_num_cast("submission_history.used_attempts", row.used_attempts)?,
            checked_num_cast(
                "student_assignment_progress.extra_attempts",
                row.extra_attempts,
            )?,
            row.has_accepted_submission,
            row.has_pending_submission,
        );

        Ok(Self { student, history })
    }
}
