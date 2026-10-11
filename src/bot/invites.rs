use crate::domain::{InviteGeneration, TeachingCourseId};
use crate::invites::{CourseInviteCodec, InviteDecodeError, VerifiedInvite};

#[derive(Clone)]
pub(super) struct BotInvites {
    codec: CourseInviteCodec,
    bot_username: String,
}

impl BotInvites {
    pub(super) fn new(codec: CourseInviteCodec, bot_username: String) -> Self {
        Self {
            codec,
            bot_username,
        }
    }

    pub(super) fn decode(&self, payload: &str) -> Result<VerifiedInvite, InviteDecodeError> {
        self.codec.decode(payload)
    }

    pub(super) fn link(&self, course_id: TeachingCourseId, generation: InviteGeneration) -> String {
        format!(
            "https://t.me/{}?start={}",
            self.bot_username,
            self.codec.encode(course_id, generation)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invites::InviteSigningSecret;

    #[test]
    fn the_invite_link_carries_the_bot_username_from_get_me() {
        let codec = CourseInviteCodec::new(
            InviteSigningSecret::from_base64url("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8")
                .unwrap(),
        );
        let invites = BotInvites::new(codec, "hw_test_bot".to_owned());
        let course = TeachingCourseId::new();
        let link = invites.link(course, InviteGeneration::FIRST);

        let payload = link
            .strip_prefix("https://t.me/hw_test_bot?start=")
            .unwrap();
        assert_eq!(invites.decode(payload).unwrap().course_id(), course);
    }
}
