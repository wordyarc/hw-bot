use crate::domain::UserId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Actor {
    user_id: UserId,
}

impl Actor {
    pub fn new(user_id: UserId) -> Self {
        Self { user_id }
    }

    pub fn user_id(self) -> UserId {
        self.user_id
    }
}
