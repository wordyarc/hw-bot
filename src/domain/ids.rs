use crate::domain::{DomainError, DomainResult};
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

uuid_id!(UserId);
uuid_id!(StudentGroupId);
uuid_id!(SubjectId);
uuid_id!(TeachingCourseId);
uuid_id!(AssignmentId);
uuid_id!(SubmissionId);
uuid_id!(ExtraAttemptGrantId);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TelegramUserId(i64);

impl TelegramUserId {
    pub fn new(value: i64) -> DomainResult<Self> {
        if value <= 0 {
            return Err(DomainError::InvalidTelegramUserId { value });
        }

        Ok(Self(value))
    }

    pub fn as_i64(self) -> i64 {
        self.0
    }
}
