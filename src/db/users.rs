use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::submissions::{history_columns, with_submission_history};
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, Role, StudentGroupId, TeachingCourseId, TelegramUserId, User, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::{AssignmentStudentSummary, CourseStudent, RosterStudent, UserStatistics};

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
    page: PageRequest,
) -> DbResult<Vec<AssignmentStudentSummary>>
where
    E: PgExecutor<'e>,
{
    with_submission_history!(
        query_as, AssignmentStudentCardRow;
        "student_id",
        "assignment_id = $1",
        r#"
        users.id,
        users.telegram_user_id,
        users.role,
        users.display_name,
        users.student_group_id,
        users.registered_at,
        student_groups.code as group_code
        "#,
        r#"
        from users
        join assignment_groups issued
          on issued.assignment_id = $1
         and issued.student_group_id = users.student_group_id
        join student_groups
          on student_groups.id = users.student_group_id
        left join history
          on history.student_id = users.id
        left join student_assignment_progress progress
          on progress.assignment_id = $1
         and progress.student_id = users.id
        where users.role = 'student'
        order by users.display_name, users.id
        limit $2 offset $3
        "#,
        assignment_id.as_uuid(),
        page.fetch_limit(),
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
                join assignment_groups issued
                  on issued.assignment_id = assignments.id
                where issued.student_group_id = $2
                  and assignments.status = 'published'
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
        page.fetch_limit(),
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

#[derive(Clone, Debug)]
pub struct CourseRosterRow {
    pub group_code: String,
    pub group_size: u32,
    pub issued: u32,
    pub student: Option<RosterStudent>,
}

pub async fn course_roster<'e, E>(
    executor: E,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> DbResult<Vec<CourseRosterRow>>
where
    E: PgExecutor<'e>,
{
    sqlx::query!(
        r#"
        with issued as (
            select issued.student_group_id, issued.assignment_id
            from assignment_groups issued
            join assignments on assignments.id = issued.assignment_id
            where issued.teaching_course_id = $1
              and assignments.published_at is not null
        ), accepted as (
            select submissions.student_id,
                   count(distinct submissions.assignment_id) as assignments
            from submissions
            join users on users.id = submissions.student_id
            join issued
              on issued.assignment_id = submissions.assignment_id
             and issued.student_group_id = users.student_group_id
            where submissions.status = 'accepted'
            group by submissions.student_id
        )
        select
            student_groups.code as group_code,
            count(users.id) over (partition by student_groups.id) as "group_size!",
            row_number() over (
                partition by student_groups.id order by users.display_name, users.id
            ) as "position!",
            (
                select count(*) from issued
                where issued.student_group_id = student_groups.id
            ) as "issued!",
            users.id as "student_id?",
            users.display_name as "display_name?",
            coalesce(accepted.assignments, 0) as "accepted!"
        from teaching_course_groups course_groups
        join student_groups on student_groups.id = course_groups.student_group_id
        left join users
          on users.student_group_id = student_groups.id
         and users.role = 'student'
        left join accepted on accepted.student_id = users.id
        where course_groups.teaching_course_id = $1
        order by student_groups.code, users.display_name, users.id
        limit $2 offset $3
        "#,
        course_id.as_uuid(),
        page.fetch_limit(),
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(|row| {
        let student = row
            .student_id
            .zip(row.display_name)
            .map(|(id, display_name)| {
                Ok::<_, DbError>(RosterStudent {
                    position: checked_num_cast("roster.position", row.position)?,
                    student_id: UserId::from_uuid(id),
                    display_name,
                    accepted: checked_num_cast("roster.accepted", row.accepted)?,
                })
            })
            .transpose()?;

        Ok(CourseRosterRow {
            group_code: row.group_code,
            group_size: checked_num_cast("roster.group_size", row.group_size)?,
            issued: checked_num_cast("roster.issued", row.issued)?,
            student,
        })
    })
    .collect()
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
    group_code: String,
    used_attempts: i64,
    submission_count: i64,
    extra_attempts: i16,
    has_accepted_regular: bool,
    accepted_regular_score: Option<i16>,
    has_accepted_makeup: bool,
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
        let history = history_columns!(row).history()?;

        Ok(Self {
            student,
            group_code: row.group_code,
            history,
        })
    }
}
