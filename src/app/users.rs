use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;

use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    DomainError, DomainResult, StaffRole, StudentGroup, StudentGroupId, Subject, TelegramUserId,
    User, UserId,
};
use crate::invites::VerifiedInvite;
use crate::read_models::UserStatistics;

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
    pub student_group_id: Option<StudentGroupId>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct CourseInvitation {
    pub subject: Subject,
    pub groups: Vec<StudentGroup>,
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

    pub async fn statistics(&self, actor: Actor) -> AppResult<UserStatistics> {
        let user = load_actor(&self.pool, actor, "user").await?;

        Ok(db::users::profile_statistics(&self.pool, &user).await?)
    }

    /// Проверка под `for share` и добавление студента входят в одну транзакцию.
    /// Если после `/start` ссылку сменили, ввод имени уже не завершит регистрацию по ней.
    pub async fn register_student(&self, command: RegisterStudent) -> AppResult<RegisteredUser> {
        let mut tx = self.pool.begin().await?;

        let course = db::course_invites::lock_course_for_registration(
            &mut tx,
            command.invite.course_id(),
            command.invite.generation(),
        )
        .await?
        .ok_or(AppError::InviteInvalid)?;
        let student_group = chosen_group(course.groups, command.student_group_id)?;

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
            Some(id) => Some(
                db::academic::find_student_group_by_id(&self.pool, id)
                    .await?
                    .required("student group")?,
            ),
            None => None,
        };

        Ok(RegisteredUser {
            user,
            student_group,
        })
    }

    pub async fn resolve_course_invite(
        &self,
        invite: VerifiedInvite,
    ) -> AppResult<CourseInvitation> {
        let generation =
            db::course_invites::find_generation(&self.pool, invite.course_id()).await?;
        if generation != Some(invite.generation()) {
            return Err(AppError::InviteInvalid);
        }

        let course = db::academic::find_teaching_course_summary(&self.pool, invite.course_id())
            .await?
            .ok_or(AppError::InviteInvalid)?;

        Ok(CourseInvitation {
            subject: course.subject,
            groups: course.groups,
        })
    }
}

fn chosen_group(
    groups: Vec<StudentGroup>,
    chosen: Option<StudentGroupId>,
) -> DomainResult<StudentGroup> {
    match chosen {
        Some(group_id) => groups
            .into_iter()
            .find(|group| group.id() == group_id)
            .ok_or(DomainError::StudentGroupNotInCourse),
        None => match <[StudentGroup; 1]>::try_from(groups) {
            Ok([group]) => Ok(group),
            Err(_) => Err(DomainError::StudentGroupChoiceRequired),
        },
    }
}
