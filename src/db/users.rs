use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx::postgres::PgExecutor;
use uuid::Uuid;

use crate::db::{DbError, DbResult};
use crate::domain::{Role, StudentGroupId, TelegramUserId, User, UserId};

#[derive(Clone, Debug, Default)]
pub struct UserRepository;

impl UserRepository {
    pub fn new() -> Self {
        Self
    }

    pub async fn find_by_id<'e, E>(&self, executor: E, id: UserId) -> DbResult<Option<User>>
    where
        E: PgExecutor<'e>,
    {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            select
                id,
                telegram_user_id,
                role,
                display_name,
                student_group_id,
                registered_at
            from users
            where id = $1
            "#,
        )
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
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            select
                id,
                telegram_user_id,
                role,
                display_name,
                student_group_id,
                registered_at
            from users
            where telegram_user_id = $1
            "#,
        )
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
        let role = role_from_db(&row.role, row.student_group_id)?;

        Ok(User::new(
            UserId::from_uuid(row.id),
            telegram_user_id,
            role,
            row.display_name,
            row.registered_at,
        )?)
    }
}

/// The `student` role carries its group, so the two database columns collapse into one enum. A
/// mismatch between them is corrupted data rather than a domain rule violation.
fn role_from_db(value: &str, student_group_id: Option<Uuid>) -> DbResult<Role> {
    let invalid_group = || DbError::InvalidDatabaseValue {
        field: "users.student_group_id",
        value: format!("{student_group_id:?} for role {value}"),
    };

    match (value, student_group_id) {
        ("student", Some(group_id)) => Ok(Role::Student {
            student_group_id: StudentGroupId::from_uuid(group_id),
        }),
        ("teacher", None) => Ok(Role::Teacher),
        ("admin", None) => Ok(Role::Admin),
        ("student" | "teacher" | "admin", _) => Err(invalid_group()),
        other => Err(DbError::InvalidDatabaseValue {
            field: "users.role",
            value: other.0.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_roles_from_database_values() {
        let group_id = StudentGroupId::new();

        assert_eq!(
            role_from_db("student", Some(group_id.as_uuid())).unwrap(),
            Role::Student {
                student_group_id: group_id
            }
        );
        assert_eq!(role_from_db("teacher", None).unwrap(), Role::Teacher);
        assert_eq!(role_from_db("admin", None).unwrap(), Role::Admin);
    }

    #[test]
    fn rejects_role_inconsistent_with_student_group() {
        assert!(matches!(
            role_from_db("student", None),
            Err(DbError::InvalidDatabaseValue {
                field: "users.student_group_id",
                ..
            })
        ));
        assert!(matches!(
            role_from_db("teacher", Some(Uuid::new_v4())),
            Err(DbError::InvalidDatabaseValue {
                field: "users.student_group_id",
                ..
            })
        ));
    }

    #[test]
    fn rejects_unknown_role() {
        assert!(matches!(
            role_from_db("unknown", None),
            Err(DbError::InvalidDatabaseValue {
                field: "users.role",
                ..
            })
        ));
    }
}
