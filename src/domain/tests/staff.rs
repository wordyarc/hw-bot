use super::{Fixture, fixture};
use crate::domain::{DomainError, Role, StaffRole, TelegramUserId, User, UserId};
use crate::test_support::{admin, t0};

#[test]
fn only_admin_can_register_staff() {
    let Fixture {
        teacher, student, ..
    } = fixture();
    let admin = admin(3);

    for actor in [&teacher, &student] {
        assert_eq!(
            User::new_staff(
                UserId::new(),
                TelegramUserId::new(100).unwrap(),
                StaffRole::Teacher,
                "New teacher",
                actor,
                t0(),
            )
            .unwrap_err(),
            DomainError::PermissionDenied {
                action: "register staff"
            }
        );
    }

    let registered = User::new_staff(
        UserId::new(),
        TelegramUserId::new(100).unwrap(),
        StaffRole::Teacher,
        "New teacher",
        &admin,
        t0(),
    )
    .unwrap();

    assert_eq!(registered.role(), Role::Teacher);
    assert_eq!(
        registered.student_group_id(),
        None,
        "staff is never bound to a student group"
    );
}
