use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use sqlx::postgres::PgExecutor;

use crate::app::access::{load_student, lock_student};
use crate::app::{Actor, AppError, AppResult, Required};
use crate::db;
use crate::domain::{
    DomainError, DomainResult, StaffRole, StudentGroup, StudentGroupId, Subject, TeachingCourseId,
    TelegramUserId, User, UserId,
};
use crate::invites::VerifiedInvite;
use crate::read_models::{OfferedCourse, UserStatistics};

pub(crate) async fn load_actor(
    executor: impl PgExecutor<'_>,
    actor: Actor,
    resource: &'static str,
) -> AppResult<User> {
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
    pub courses: Option<BTreeSet<TeachingCourseId>>,
    pub registered_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct CourseInvitation {
    pub subject: Subject,
    pub groups: Vec<StudentGroup>,
}

#[derive(Clone, Debug)]
pub struct RegistrationOffer {
    pub group: StudentGroup,
    pub courses: Vec<OfferedCourse>,
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
        let offered = db::enrollments::offered_courses(&mut *tx, student_group.id(), None).await?;
        let courses = chosen_courses(&offered, command.courses, command.invite.course_id())?;

        let student = User::new_student(
            UserId::new(),
            command.telegram_user_id,
            command.display_name,
            &student_group,
            command.registered_at,
        )?;

        db::users::insert(&mut *tx, &student).await?;
        db::enrollments::insert(&mut *tx, student.id(), &courses).await?;

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

    pub async fn find_registered(
        &self,
        telegram_user_id: TelegramUserId,
    ) -> AppResult<RegisteredUser> {
        let user = self.find_user(telegram_user_id).await?;
        self.registered_user(user).await
    }

    pub async fn find_user(&self, telegram_user_id: TelegramUserId) -> AppResult<User> {
        db::users::find_by_telegram_user_id(&self.pool, telegram_user_id)
            .await?
            .required("user")
    }

    pub async fn resolve_actor(&self, telegram_user_id: TelegramUserId) -> AppResult<Actor> {
        let user = self.find_user(telegram_user_id).await?;

        Ok(Actor::new(user.id()))
    }

    pub async fn find_registered_by_actor(&self, actor: Actor) -> AppResult<RegisteredUser> {
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

        let course = db::academic::find_teaching_course_details(&self.pool, invite.course_id())
            .await?
            .ok_or(AppError::InviteInvalid)?;

        Ok(CourseInvitation {
            subject: course.subject,
            groups: course.groups,
        })
    }

    pub async fn registration_offer(
        &self,
        invite: VerifiedInvite,
        group_id: StudentGroupId,
    ) -> AppResult<RegistrationOffer> {
        let invitation = self.resolve_course_invite(invite).await?;
        let group = chosen_group(invitation.groups, Some(group_id))?;
        let courses = db::enrollments::offered_courses(&self.pool, group.id(), None).await?;

        Ok(RegistrationOffer { group, courses })
    }

    pub async fn enrollment_offer(
        &self,
        actor: Actor,
        invite: VerifiedInvite,
    ) -> AppResult<Option<Vec<OfferedCourse>>> {
        let user = load_actor(&self.pool, actor, "user").await?;
        let Some(group_id) = user.student_group_id() else {
            return Ok(None);
        };
        let generation =
            db::course_invites::find_generation(&self.pool, invite.course_id()).await?;
        if generation != Some(invite.generation()) {
            return Ok(None);
        }

        let offered =
            db::enrollments::offered_courses(&self.pool, group_id, Some(user.id())).await?;
        Ok(offered
            .iter()
            .any(|course| course.course_id == invite.course_id())
            .then_some(offered))
    }

    pub async fn open_courses(&self, actor: Actor) -> AppResult<Vec<OfferedCourse>> {
        let student = load_student(&self.pool, actor, "read courses to enroll in").await?;
        let group_id = student.student_group_id().required("student group")?;

        Ok(db::enrollments::offered_courses(&self.pool, group_id, Some(student.id())).await?)
    }

    pub async fn enroll(
        &self,
        actor: Actor,
        courses: &BTreeSet<TeachingCourseId>,
    ) -> AppResult<Vec<OfferedCourse>> {
        let mut tx = self.pool.begin().await?;

        let student = lock_student(&mut *tx, actor, "enroll in courses").await?;
        let group_id = student.student_group_id().required("student group")?;

        let chosen: Vec<_> = db::enrollments::offered_courses(&mut *tx, group_id, None)
            .await?
            .into_iter()
            .filter(|course| courses.contains(&course.course_id))
            .collect();
        if chosen.len() != courses.len() {
            return Err(DomainError::CourseNotOfferedToStudentGroup.into());
        }

        let course_ids: Vec<_> = chosen.iter().map(|course| course.course_id).collect();
        db::enrollments::insert(&mut *tx, student.id(), &course_ids).await?;

        tx.commit().await?;

        Ok(chosen)
    }

    pub async fn subjects(&self, actor: Actor) -> AppResult<Vec<Subject>> {
        Ok(db::enrollments::subjects_of_student(&self.pool, actor.user_id()).await?)
    }
}

fn chosen_courses(
    offered: &[OfferedCourse],
    chosen: Option<BTreeSet<TeachingCourseId>>,
    invited: TeachingCourseId,
) -> DomainResult<Vec<TeachingCourseId>> {
    let is_offered = |id: &TeachingCourseId| offered.iter().any(|course| course.course_id == *id);

    match chosen {
        Some(chosen) if chosen.iter().all(is_offered) => Ok(chosen.into_iter().collect()),
        Some(_) => Err(DomainError::CourseNotOfferedToStudentGroup),
        None => Ok(Some(invited).into_iter().filter(is_offered).collect()),
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
