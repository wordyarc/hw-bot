use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;

use crate::app::{Actor, AppResult, Required};
use crate::db;
use crate::domain::{StaffRole, StudentGroup, TelegramUserId, User, UserId};

pub(crate) async fn load_actor<'e, E>(
    executor: E,
    actor: Actor,
    resource: &'static str,
) -> AppResult<User>
where
    E: PgExecutor<'e>,
{
    db::users::find_by_id(executor, actor.user_id())
        .await?
        .required(resource)
}

#[derive(Clone, Debug)]
pub struct RegisterStudent {
    pub telegram_user_id: TelegramUserId,
    pub display_name: String,
    pub student_group_code: String,
    pub registered_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct RegisterStaff {
    pub actor: Actor,
    pub telegram_user_id: TelegramUserId,
    pub display_name: String,
    pub role: StaffRole,
    pub registered_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct RegisteredUser {
    pub user: User,
    pub student_group: Option<StudentGroup>,
}

#[derive(Clone, Debug)]
pub struct UserService {
    pool: PgPool,
}

impl UserService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn register_student(&self, command: RegisterStudent) -> AppResult<RegisteredUser> {
        let mut tx = self.pool.begin().await?;

        let student_group =
            db::academic::find_student_group_by_code(&mut *tx, &command.student_group_code)
                .await?
                .required("student group")?;

        let student = User::new_student(
            UserId::new(),
            command.telegram_user_id,
            command.display_name,
            &student_group,
            command.registered_at,
        )?;

        db::users::insert(&mut *tx, &student).await?;

        tx.commit().await?;

        Ok(RegisteredUser {
            user: student,
            student_group: Some(student_group),
        })
    }

    pub async fn register_staff(&self, command: RegisterStaff) -> AppResult<RegisteredUser> {
        let mut tx = self.pool.begin().await?;

        let actor = load_actor(&mut *tx, command.actor, "actor").await?;

        let staff = User::new_staff(
            UserId::new(),
            command.telegram_user_id,
            command.role,
            command.display_name,
            &actor,
            command.registered_at,
        )?;

        db::users::insert(&mut *tx, &staff).await?;

        tx.commit().await?;

        Ok(RegisteredUser {
            user: staff,
            student_group: None,
        })
    }

    pub async fn find_by_telegram_user_id(
        &self,
        telegram_user_id: TelegramUserId,
    ) -> AppResult<RegisteredUser> {
        let user = db::users::find_by_telegram_user_id(&self.pool, telegram_user_id)
            .await?
            .required("user")?;
        self.registered_user(user).await
    }

    pub async fn resolve_actor(&self, telegram_user_id: TelegramUserId) -> AppResult<Actor> {
        let user = db::users::find_by_telegram_user_id(&self.pool, telegram_user_id)
            .await?
            .required("user")?;

        Ok(Actor::new(user.id()))
    }

    pub async fn find_by_actor(&self, actor: Actor) -> AppResult<RegisteredUser> {
        let user = load_actor(&self.pool, actor, "user").await?;
        self.registered_user(user).await
    }

    async fn registered_user(&self, user: User) -> AppResult<RegisteredUser> {
        let student_group = match user.student_group_id() {
            Some(id) => db::academic::find_student_group_by_id(&self.pool, id)
                .await?
                .required("student group")
                .map(Some)?,
            None => None,
        };

        Ok(RegisteredUser {
            user,
            student_group,
        })
    }

    pub async fn list_student_groups(&self) -> AppResult<Vec<StudentGroup>> {
        Ok(db::academic::list_student_groups(&self.pool).await?)
    }
}
