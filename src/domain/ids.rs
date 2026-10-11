use uuid::Uuid;

use crate::domain::{DomainError, DomainResult};

macro_rules! uuid_id {
    ($name:ident, $generator:expr) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self($generator())
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

uuid_id!(UserId, Uuid::now_v7);
uuid_id!(StudentGroupId, Uuid::new_v4);
uuid_id!(SubjectId, Uuid::new_v4);
uuid_id!(TeachingCourseId, Uuid::now_v7);
uuid_id!(AssignmentId, Uuid::now_v7);
uuid_id!(SubmissionId, Uuid::now_v7);
uuid_id!(StoredFileId, Uuid::now_v7);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_heavy_entity_ids_are_uuid_v7() {
        assert_eq!(UserId::new().as_uuid().get_version_num(), 7);
        assert_eq!(TeachingCourseId::new().as_uuid().get_version_num(), 7);
        assert_eq!(AssignmentId::new().as_uuid().get_version_num(), 7);
        assert_eq!(SubmissionId::new().as_uuid().get_version_num(), 7);
        assert_eq!(StoredFileId::new().as_uuid().get_version_num(), 7);
    }

    #[test]
    fn low_write_dictionary_ids_remain_uuid_v4() {
        assert_eq!(StudentGroupId::new().as_uuid().get_version_num(), 4);
        assert_eq!(SubjectId::new().as_uuid().get_version_num(), 4);
    }

    #[test]
    fn typed_ids_can_be_restored_from_uuid() {
        let uuid = Uuid::now_v7();

        assert_eq!(AssignmentId::from_uuid(uuid).as_uuid(), uuid);
    }

    #[test]
    fn telegram_user_id_must_be_positive() {
        assert_eq!(
            TelegramUserId::new(0).unwrap_err(),
            DomainError::InvalidTelegramUserId { value: 0 }
        );
        assert_eq!(TelegramUserId::new(42).unwrap().as_i64(), 42);
    }
}
