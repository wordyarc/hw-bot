use crate::domain::{InviteGeneration, StudentGroupId};
use crate::invites::{GroupInviteCodec, InviteDecodeError, VerifiedInvite};

#[derive(Clone)]
pub(super) struct BotInvites {
    codec: GroupInviteCodec,
    bot_username: String,
}

impl BotInvites {
    pub(super) fn new(codec: GroupInviteCodec, bot_username: String) -> Self {
        Self {
            codec,
            bot_username,
        }
    }

    pub(super) fn decode(&self, payload: &str) -> Result<VerifiedInvite, InviteDecodeError> {
        self.codec.decode(payload)
    }

    pub(super) fn link(&self, group_id: StudentGroupId, generation: InviteGeneration) -> String {
        format!(
            "https://t.me/{}?start={}",
            self.bot_username,
            self.codec.encode(group_id, generation)
        )
    }
}
