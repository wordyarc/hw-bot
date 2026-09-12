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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invites::InviteSigningSecret;

    #[test]
    fn the_invite_link_carries_the_bot_username_from_get_me() {
        let codec = GroupInviteCodec::new(
            InviteSigningSecret::from_base64url("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8")
                .unwrap(),
        );
        let invites = BotInvites::new(codec, "hw_test_bot".to_owned());
        let group = StudentGroupId::new();
        let link = invites.link(group, InviteGeneration::FIRST);

        let payload = link
            .strip_prefix("https://t.me/hw_test_bot?start=")
            .unwrap();
        assert_eq!(invites.decode(payload).unwrap().group_id(), group);
    }
}
