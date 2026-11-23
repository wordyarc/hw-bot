use chrono::{DateTime, Utc};
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::submissions::{history_columns, with_submission_history};
use crate::db::{DbError, DbResult, checked_num_cast};
use crate::domain::{
    AssignmentId, Role, StudentGroupId, TeachingCourseId, TelegramUserId, User, UserId,
};
use crate::pagination::PageRequest;
use crate::read_models::{
    AssignmentStudentSummary, CourseStudent, RosterStudent, StudentListItem, UserStatistics,
};

pub async fn insert(executor: impl PgExecutor<'_>, user: &User) -> DbResult<()> {
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

pub async fn any_exist(executor: impl PgExecutor<'_>) -> DbResult<bool> {
    Ok(
        sqlx::query_scalar!(r#"select exists (select 1 from users) as "exists!""#)
            .fetch_one(executor)
            .await?,
    )
}

pub async fn find_by_id(executor: impl PgExecutor<'_>, id: UserId) -> DbResult<Option<User>> {
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

pub(crate) async fn find_for_key_share(
    executor: impl PgExecutor<'_>,
    id: UserId,
) -> DbResult<Option<User>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, telegram_user_id, role, display_name, student_group_id, registered_at
        from users
        where id = $1
        for key share
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(User::try_from).transpose()
}

pub(crate) async fn find_for_update(
    executor: impl PgExecutor<'_>,
    id: UserId,
) -> DbResult<Option<User>> {
    let row = sqlx::query_as!(
        UserRow,
        r#"
        select id, telegram_user_id, role, display_name, student_group_id, registered_at
        from users
        where id = $1
        for update
        "#,
        id.as_uuid(),
    )
    .fetch_optional(executor)
    .await?;

    row.map(User::try_from).transpose()
}

pub(crate) async fn update_display_name(
    executor: impl PgExecutor<'_>,
    id: UserId,
    display_name: &str,
) -> DbResult<()> {
    sqlx::query!(
        "update users set display_name = $2 where id = $1",
        id.as_uuid(),
        display_name,
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub(crate) async fn update_student_group(
    executor: impl PgExecutor<'_>,
    id: UserId,
    group_id: StudentGroupId,
) -> DbResult<()> {
    sqlx::query!(
        "update users set student_group_id = $2 where id = $1 and role = 'student'",
        id.as_uuid(),
        group_id.as_uuid(),
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub(crate) async fn count_blocking_submissions(
    executor: impl PgExecutor<'_>,
    student_id: UserId,
    course_id: Option<TeachingCourseId>,
) -> DbResult<u32> {
    let blocking = sqlx::query_scalar!(
        r#"
        select count(*) as "blocking!"
        from submissions
        join assignments
          on assignments.id = submissions.assignment_id
        where submissions.student_id = $1
          and submissions.status <> 'withdrawn'
          and ($2::uuid is null or assignments.teaching_course_id = $2)
        "#,
        student_id.as_uuid(),
        course_id.map(TeachingCourseId::as_uuid),
    )
    .fetch_one(executor)
    .await?;

    checked_num_cast("admin.blocking_submissions", blocking)
}

pub(crate) async fn search_students(
    executor: impl PgExecutor<'_>,
    pattern: &str,
    page: PageRequest,
) -> DbResult<Vec<StudentListItem>> {
    let rows = sqlx::query!(
        r#"
        select users.id, users.display_name, student_groups.code as group_code
        from users
        join student_groups
          on student_groups.id = users.student_group_id
        where users.role = 'student'
          and replace(lower(users.display_name), 'ё', 'е') like $1 escape '\'
        order by users.display_name, users.id
        limit $2 offset $3
        "#,
        pattern,
        page.fetch_limit(),
        page.offset(),
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| StudentListItem {
            student_id: UserId::from_uuid(row.id),
            display_name: row.display_name,
            group_code: row.group_code,
        })
        .collect())
}

pub(crate) async fn list_staff(
    executor: impl PgExecutor<'_>,
    page: PageRequest,
) -> DbResult<Vec<User>> {
    sqlx::query_as!(
        UserRow,
        r#"
        select id, telegram_user_id, role, display_name, student_group_id, registered_at
        from users
        where role in ('teacher', 'admin')
        order by display_name, id
        limit $1 offset $2
        "#,
        page.fetch_limit(),
        page.offset(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(User::try_from)
    .collect()
}

pub(crate) async fn list_course_teachers(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
) -> DbResult<Vec<User>> {
    sqlx::query_as!(
        UserRow,
        r#"
        select
            users.id, users.telegram_user_id, users.role, users.display_name,
            users.student_group_id, users.registered_at
        from teaching_course_teachers course_teacher
        join users
          on users.id = course_teacher.teacher_id
        where course_teacher.teaching_course_id = $1
        order by users.display_name, users.id
        "#,
        course_id.as_uuid(),
    )
    .fetch_all(executor)
    .await?
    .into_iter()
    .map(User::try_from)
    .collect()
}

pub async fn find_by_telegram_user_id(
    executor: impl PgExecutor<'_>,
    telegram_user_id: TelegramUserId,
) -> DbResult<Option<User>> {
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

pub(crate) async fn list_students_with_assignment_history(
    executor: impl PgExecutor<'_>,
    assignment_id: AssignmentId,
    page: PageRequest,
) -> DbResult<Vec<AssignmentStudentSummary>> {
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
        join course_participants participant
          on participant.teaching_course_id = issued.teaching_course_id
         and participant.student_id = users.id
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

pub async fn profile_statistics(
    executor: impl PgExecutor<'_>,
    user: &User,
) -> DbResult<UserStatistics> {
    let row = sqlx::query!(
        r#"
        select
            (
                select count(*)
                from assignments
                join course_participants participant
                  on participant.teaching_course_id = assignments.teaching_course_id
                 and participant.student_id = $1
                join assignment_groups issued
                  on issued.assignment_id = assignments.id
                 and issued.student_group_id = participant.student_group_id
                where assignments.status = 'published'
            ) as "assignments!",
            (
                select count(*)
                from submissions
                where student_id = $1
                  and status = 'pending_review'
            ) as "pending!",
            (
                select count(*)
                from teaching_course_teachers teacher
                where teacher.teacher_id = $1
            ) as "courses!"
        "#,
        user.id().as_uuid(),
    )
    .fetch_one(executor)
    .await?;

    Ok(UserStatistics {
        assignments: checked_num_cast("profile.assignments", row.assignments)?,
        pending: checked_num_cast("profile.pending", row.pending)?,
        courses: checked_num_cast("profile.courses", row.courses)?,
    })
}

pub async fn list_student_names(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    group_id: StudentGroupId,
    page: PageRequest,
) -> DbResult<Vec<CourseStudent>> {
    let rows = sqlx::query!(
        r#"
        select users.id, users.display_name
        from course_participants participant
        join users
          on users.id = participant.student_id
        where participant.teaching_course_id = $1
          and participant.student_group_id = $2
        order by users.display_name, users.id
        limit $3 offset $4
        "#,
        course_id.as_uuid(),
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

pub async fn course_roster(
    executor: impl PgExecutor<'_>,
    course_id: TeachingCourseId,
    page: PageRequest,
) -> DbResult<Vec<CourseRosterRow>> {
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
            users.display_name as "display_name?",
            coalesce(accepted.assignments, 0) as "accepted!"
        from teaching_course_groups course_groups
        join student_groups on student_groups.id = course_groups.student_group_id
        left join course_participants participant
          on participant.teaching_course_id = course_groups.teaching_course_id
         and participant.student_group_id = student_groups.id
        left join users on users.id = participant.student_id
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
            .display_name
            .map(|display_name| {
                Ok::<_, DbError>(RosterStudent {
                    position: checked_num_cast("roster.position", row.position)?,
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
