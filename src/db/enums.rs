use uuid::Uuid;

use crate::db::{DbError, DbResult};
use crate::domain::{Role, StudentGroupId};

impl Role {
    /// Роль размазана по двум колонкам, поэтому у нее собственный маппинг, а не `str_enum!`.
    pub(crate) fn from_db_str(value: &str, student_group_id: Option<Uuid>) -> DbResult<Self> {
        let invalid_group = || {
            DbError::invalid_value(
                "users.student_group_id",
                format!("{student_group_id:?} for role {value}"),
            )
        };

        match (value, student_group_id) {
            ("student", Some(group_id)) => Ok(Self::Student {
                student_group_id: StudentGroupId::from_uuid(group_id),
            }),
            ("teacher", None) => Ok(Self::Teacher),
            ("admin", None) => Ok(Self::Admin),
            ("student" | "teacher" | "admin", _) => Err(invalid_group()),
            other => Err(DbError::invalid_value("users.role", other.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                Role::from_db_str(role.as_str(), group_column).unwrap(),
                role
            );
        }
    }

    #[test]
    fn rejects_unknown_roles() {
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
