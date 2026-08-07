use uuid::Uuid;

use crate::db::{DbError, DbResult};
use crate::domain::{AssignmentStatus, Role, StudentGroupId, SubmissionStatus};

macro_rules! db_enum {
    ($type:ty, $column:literal { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $type {
            pub fn as_db_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub(crate) fn from_db_str(value: &str) -> DbResult<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(DbError::InvalidDatabaseValue {
                        field: $column,
                        value: other.to_owned(),
                    }),
                }
            }
        }
    };
}

db_enum!(SubmissionStatus, "submissions.status" {
    PendingReview => "pending_review",
    Accepted => "accepted",
    Rejected => "rejected",
});

db_enum!(AssignmentStatus, "assignments.status" {
    Draft => "draft",
    Published => "published",
    Archived => "archived",
});

impl Role {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Student { .. } => "student",
            Self::Teacher => "teacher",
            Self::Admin => "admin",
        }
    }

    pub(crate) fn from_db_str(value: &str, student_group_id: Option<Uuid>) -> DbResult<Self> {
        let invalid_group = || DbError::InvalidDatabaseValue {
            field: "users.student_group_id",
            value: format!("{student_group_id:?} for role {value}"),
        };

        match (value, student_group_id) {
            ("student", Some(group_id)) => Ok(Self::Student {
                student_group_id: StudentGroupId::from_uuid(group_id),
            }),
            ("teacher", None) => Ok(Self::Teacher),
            ("admin", None) => Ok(Self::Admin),
            ("student" | "teacher" | "admin", _) => Err(invalid_group()),
            other => Err(DbError::InvalidDatabaseValue {
                field: "users.role",
                value: other.0.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_statuses_survive_a_round_trip() {
        for status in [
            SubmissionStatus::PendingReview,
            SubmissionStatus::Accepted,
            SubmissionStatus::Rejected,
        ] {
            assert_eq!(
                SubmissionStatus::from_db_str(status.as_db_str()).unwrap(),
                status
            );
        }
    }

    #[test]
    fn assignment_statuses_survive_a_round_trip() {
        for status in [
            AssignmentStatus::Draft,
            AssignmentStatus::Published,
            AssignmentStatus::Archived,
        ] {
            assert_eq!(
                AssignmentStatus::from_db_str(status.as_db_str()).unwrap(),
                status
            );
        }
    }

    #[test]
    fn roles_survive_a_round_trip() {
        let group_id = StudentGroupId::new();

        for role in [
            Role::Student {
                student_group_id: group_id,
            },
            Role::Teacher,
            Role::Admin,
        ] {
            let group_column = role.student_group_id().map(StudentGroupId::as_uuid);

            assert_eq!(
                Role::from_db_str(role.as_db_str(), group_column).unwrap(),
                role
            );
        }
    }

    #[test]
    fn rejects_unknown_statuses() {
        assert!(matches!(
            SubmissionStatus::from_db_str("unknown"),
            Err(DbError::InvalidDatabaseValue {
                field: "submissions.status",
                ..
            })
        ));
        assert!(matches!(
            AssignmentStatus::from_db_str("unknown"),
            Err(DbError::InvalidDatabaseValue {
                field: "assignments.status",
                ..
            })
        ));
        assert!(matches!(
            Role::from_db_str("unknown", None),
            Err(DbError::InvalidDatabaseValue {
                field: "users.role",
                ..
            })
        ));
    }

    #[test]
    fn rejects_role_inconsistent_with_student_group() {
        assert!(matches!(
            Role::from_db_str("student", None),
            Err(DbError::InvalidDatabaseValue {
                field: "users.student_group_id",
                ..
            })
        ));
        assert!(matches!(
            Role::from_db_str("teacher", Some(Uuid::new_v4())),
            Err(DbError::InvalidDatabaseValue {
                field: "users.student_group_id",
                ..
            })
        ));
    }
}
