use chrono::{DateTime, Utc};

use crate::domain::enums::str_enum;
use crate::domain::{
    DISPLAY_NAME_MAX_LEN, DomainError, DomainResult, NonEmptyString, StudentGroup, StudentGroupId,
    TelegramUserId, UserId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Student { student_group_id: StudentGroupId },
    Teacher,
    Admin,
}

str_enum! {
    /// Роли, которые нельзя выдать себе самому: студент регистрируется сам, персонал заводит админ.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum StaffRole {
        Teacher => "teacher",
        Admin => "admin",
    }
}

impl From<StaffRole> for Role {
    fn from(role: StaffRole) -> Self {
        match role {
            StaffRole::Teacher => Self::Teacher,
            StaffRole::Admin => Self::Admin,
        }
    }
}

impl Role {
    /// Имя роли одно и то же в БД и в wire-формате API, как и у плоских `str_enum!`-ов.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Student { .. } => "student",
            Self::Teacher => "teacher",
            Self::Admin => "admin",
        }
    }

    pub fn can_submit_homework(self) -> bool {
        matches!(self, Self::Student { .. })
    }

    pub fn can_review_homework(self) -> bool {
        matches!(self, Self::Teacher | Self::Admin)
    }

    pub fn can_manage_assignments(self) -> bool {
        matches!(self, Self::Teacher | Self::Admin)
    }

    pub fn can_manage_all_teaching_courses(self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn can_grant_attempts(self) -> bool {
        matches!(self, Self::Teacher | Self::Admin)
    }

    pub fn can_manage_users(self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn student_group_id(self) -> Option<StudentGroupId> {
        match self {
            Self::Student { student_group_id } => Some(student_group_id),
            Self::Teacher | Self::Admin => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct User {
    id: UserId,
    telegram_user_id: TelegramUserId,
    role: Role,
    display_name: NonEmptyString,
    registered_at: DateTime<Utc>,
}

impl User {
    pub fn new(
        id: UserId,
        telegram_user_id: TelegramUserId,
        role: Role,
        display_name: impl Into<String>,
        registered_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Ok(Self {
            id,
            telegram_user_id,
            role,
            display_name: NonEmptyString::new_bounded(
                "user.display_name",
                display_name,
                DISPLAY_NAME_MAX_LEN,
            )?,
            registered_at,
        })
    }

    pub fn new_student(
        id: UserId,
        telegram_user_id: TelegramUserId,
        display_name: impl Into<String>,
        student_group: &StudentGroup,
        registered_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        Ok(Self {
            id,
            telegram_user_id,
            role: Role::Student {
                student_group_id: student_group.id(),
            },
            display_name: NonEmptyString::new_bounded(
                "user.display_name",
                display_name,
                DISPLAY_NAME_MAX_LEN,
            )?,
            registered_at,
        })
    }

    pub fn new_staff(
        id: UserId,
        telegram_user_id: TelegramUserId,
        role: StaffRole,
        display_name: impl Into<String>,
        actor: &User,
        registered_at: DateTime<Utc>,
    ) -> DomainResult<Self> {
        if !actor.role().can_manage_users() {
            return Err(DomainError::PermissionDenied {
                action: "register staff",
            });
        }

        Self::new(
            id,
            telegram_user_id,
            role.into(),
            display_name,
            registered_at,
        )
    }

    pub fn id(&self) -> UserId {
        self.id
    }

    pub fn telegram_user_id(&self) -> TelegramUserId {
        self.telegram_user_id
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn display_name(&self) -> &NonEmptyString {
        &self.display_name
    }

    pub fn student_group_id(&self) -> Option<StudentGroupId> {
        self.role.student_group_id()
    }

    pub fn registered_at(&self) -> DateTime<Utc> {
        self.registered_at
    }
}
