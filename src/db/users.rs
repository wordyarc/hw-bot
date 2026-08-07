use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult};
use crate::domain::{Role, StudentGroupId, TelegramUserId, User, UserId};

macro_rules! columns {
    () => {
        "id, telegram_user_id, role, display_name, student_group_id, registered_at"
    };
}

#[derive(Clone, Debug, Default)]
pub struct UserRepository;

impl UserRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn insert<'e, E>(&self, executor: E, user: &User) -> DbResult<()>
    where
        E: PgExecutor<'e>,
    {
        sqlx::query(concat!(
            "insert into users (",
            columns!(),
            ") values ($1, $2, $3, $4, $5, $6)"
        ))
        .bind(user.id().as_uuid())
        .bind(user.telegram_user_id().as_i64())
        .bind(user.role().as_db_str())
        .bind(user.display_name().as_str())
        .bind(user.student_group_id().map(StudentGroupId::as_uuid))
        .bind(user.registered_at())
        .execute(executor)
        .await?;

        Ok(())
    }

    pub async fn find_by_id<'e, E>(&self, executor: E, id: UserId) -> DbResult<Option<User>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, UserRow>(concat!(
            "select ",
            columns!(),
            " from users where id = $1"
        ))
        .bind(id.as_uuid())
        .fetch_optional(executor)
        .await?;

        row.map(User::try_from).transpose()
    }

    pub async fn find_by_telegram_user_id<'e, E>(
        &self,
        executor: E,
        telegram_user_id: TelegramUserId,
    ) -> DbResult<Option<User>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, UserRow>(concat!(
            "select ",
            columns!(),
            " from users where telegram_user_id = $1"
        ))
        .bind(telegram_user_id.as_i64())
        .fetch_optional(executor)
        .await?;

        row.map(User::try_from).transpose()
    }
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: Uuid,
    telegram_user_id: i64,
    role: String,
    display_name: String,
    student_group_id: Option<Uuid>,
    registered_at: DateTime<Utc>,
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
