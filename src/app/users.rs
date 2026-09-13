use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;

use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{StaffRole, StudentGroup, TelegramUserId, User, UserId};
use crate::invites::VerifiedInvite;

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
    pub invite: VerifiedInvite,
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

pub struct UserStatistics {
    pub assignments: u32,
    pub pending: u32,
    pub courses: u32,
}

#[derive(Clone, Debug)]
pub struct UserService {
    pool: PgPool,
}

impl UserService {
    pub async fn statistics(&self, actor: Actor) -> AppResult<UserStatistics> {
        let user = load_actor(&self.pool, actor, "user").await?;
        let (assignments, pending, courses) =
            db::users::profile_statistics(&self.pool, &user).await?;
        Ok(UserStatistics {
            assignments,
            pending,
            courses,
        })
    }

    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Инвайт перепроверяется под `for share` в той же транзакции, что и вставка студента:
    /// ссылка, заменённая между `/start` и вводом имени, регистрацию не проводит.
    pub async fn register_student(&self, command: RegisterStudent) -> AppResult<RegisteredUser> {
        let mut tx = self.pool.begin().await?;

        let student_group = db::group_invites::lock_group_for_registration(
            &mut tx,
            command.invite.group_id(),
            command.invite.generation(),
        )
        .await?
        .ok_or(AppError::InviteInvalid)?;

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

    pub async fn resolve_group_invite(&self, invite: VerifiedInvite) -> AppResult<StudentGroup> {
        let generation = db::group_invites::find_generation(&self.pool, invite.group_id()).await?;
        if generation != Some(invite.generation()) {
            return Err(AppError::InviteInvalid);
        }

        db::academic::find_student_group_by_id(&self.pool, invite.group_id())
            .await?
            .ok_or(AppError::InviteInvalid)
    }
}
