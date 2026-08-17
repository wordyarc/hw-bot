use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;

use crate::domain::{InviteGeneration, StudentGroupId};

/// Telegram ограничивает `/start` payload 64 символами.
pub const MAX_PAYLOAD_LEN: usize = 64;

const PAYLOAD_PREFIX: &str = "gi_";
const DOMAIN_SEPARATION: &[u8] = b"hw-bot/group-invite/v1";
const SECRET_LEN: usize = 32;
const TAG_LEN: usize = 16;
const GROUP_LEN: usize = 16;
const GENERATION_LEN: usize = 4;
const BODY_LEN: usize = GROUP_LEN + GENERATION_LEN + TAG_LEN;
/// Разделителя между полями нет: `-` и `_` входят в алфавит base64url, а других символов
/// Telegram в `/start` payload не принимает. Поля фиксированной длины и режутся по смещениям.
const ENCODED_BODY_LEN: usize = BODY_LEN / 3 * 4;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedInvite {
    group_id: StudentGroupId,
    generation: InviteGeneration,
}

impl VerifiedInvite {
    pub fn group_id(self) -> StudentGroupId {
        self.group_id
    }

    pub fn generation(self) -> InviteGeneration {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("group invite is not valid")]
pub struct InviteDecodeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub enum InviteSecretError {
    #[error("invite signing secret must be base64url without padding")]
    Encoding,

    #[error("invite signing secret must decode to exactly {SECRET_LEN} bytes")]
    Length,
}

#[derive(Clone)]
pub struct InviteSigningSecret([u8; SECRET_LEN]);

impl InviteSigningSecret {
    pub fn from_base64url(value: &str) -> Result<Self, InviteSecretError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(value.trim())
            .map_err(|_| InviteSecretError::Encoding)?;

        bytes
            .try_into()
            .map(Self)
            .map_err(|_| InviteSecretError::Length)
    }
}

impl fmt::Debug for InviteSigningSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InviteSigningSecret([redacted])")
    }
}

#[derive(Clone)]
pub struct GroupInviteCodec {
    secret: InviteSigningSecret,
}

impl GroupInviteCodec {
    pub fn new(secret: InviteSigningSecret) -> Self {
        Self { secret }
    }

    pub fn encode(&self, group_id: StudentGroupId, generation: InviteGeneration) -> String {
        let mut body = [0u8; BODY_LEN];
        body[..GROUP_LEN].copy_from_slice(group_id.as_uuid().as_bytes());
        body[GROUP_LEN..GROUP_LEN + GENERATION_LEN]
            .copy_from_slice(&generation.as_u32().to_be_bytes());
        body[GROUP_LEN + GENERATION_LEN..]
            .copy_from_slice(&self.tag(group_id, generation)[..TAG_LEN]);

        format!("{PAYLOAD_PREFIX}{}", URL_SAFE_NO_PAD.encode(body))
    }

    pub fn decode(&self, payload: &str) -> Result<VerifiedInvite, InviteDecodeError> {
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(InviteDecodeError);
        }

        let body = payload
            .strip_prefix(PAYLOAD_PREFIX)
            .ok_or(InviteDecodeError)?;
        if body.len() != ENCODED_BODY_LEN {
            return Err(InviteDecodeError);
        }

        let body: [u8; BODY_LEN] = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| InviteDecodeError)?
            .try_into()
            .map_err(|_| InviteDecodeError)?;
        let (group, rest) = body.split_at(GROUP_LEN);
        let (generation, tag) = rest.split_at(GENERATION_LEN);

        let group_id = StudentGroupId::from_uuid(uuid::Uuid::from_slice(group).expect("16 bytes"));
        let generation =
            InviteGeneration::new(u32::from_be_bytes(generation.try_into().expect("4 bytes")))
                .map_err(|_| InviteDecodeError)?;

        self.mac(group_id, generation)
            .verify_truncated_left(tag)
            .map_err(|_| InviteDecodeError)?;

        Ok(VerifiedInvite {
            group_id,
            generation,
        })
    }

    fn mac(&self, group_id: StudentGroupId, generation: InviteGeneration) -> HmacSha256 {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret.0).expect("HMAC accepts a key of any length");
        mac.update(DOMAIN_SEPARATION);
        mac.update(group_id.as_uuid().as_bytes());
        mac.update(&generation.as_u32().to_be_bytes());
        mac
    }

    fn tag(&self, group_id: StudentGroupId, generation: InviteGeneration) -> [u8; 32] {
        self.mac(group_id, generation)
            .finalize()
            .into_bytes()
            .into()
    }
}

impl fmt::Debug for GroupInviteCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GroupInviteCodec")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    const OTHER_SECRET: &str = "f39_f39_f39_f39_f39_f39_f39_f39_f39_f39_f38";
    const GROUP: uuid::Uuid = uuid::uuid!("2f4b7c1e-0d9a-4a6b-8c3d-5e6f70819293");

    fn codec(secret: &str) -> GroupInviteCodec {
        GroupInviteCodec::new(InviteSigningSecret::from_base64url(secret).unwrap())
    }

    fn group() -> StudentGroupId {
        StudentGroupId::from_uuid(GROUP)
    }

    #[test]
    fn payload_round_trips_through_the_codec() {
        let codec = codec(SECRET);
        let generation = InviteGeneration::new(7).unwrap();

        let decoded = codec.decode(&codec.encode(group(), generation)).unwrap();

        assert_eq!(decoded.group_id(), group());
        assert_eq!(decoded.generation(), generation);
    }

    #[test]
    fn payload_fits_the_telegram_start_limit() {
        let payload = codec(SECRET).encode(group(), InviteGeneration::new(u32::MAX).unwrap());

        assert!(payload.len() <= MAX_PAYLOAD_LEN, "{payload}");
    }

    /// Фиксирует wire-формат: расхождение здесь означает, что уже розданные ссылки перестанут
    /// работать после выкладки.
    #[test]
    fn wire_format_stays_compatible_between_versions() {
        assert_eq!(
            codec(SECRET).encode(group(), InviteGeneration::FIRST),
            "gi_L0t8Hg2aSmuMPV5vcIGSkwAAAAEdut_Q-iaKZ9reQaSSVLAI"
        );
    }

    #[test]
    fn a_different_secret_does_not_verify_the_payload() {
        let payload = codec(SECRET).encode(group(), InviteGeneration::FIRST);

        assert_eq!(codec(OTHER_SECRET).decode(&payload), Err(InviteDecodeError));
    }

    /// Мутирует уже подписанный payload побайтово, а не через `encode`: иначе тест проверял бы
    /// не подпись, а сам кодек.
    fn resign(payload: &str, mutate: impl FnOnce(&mut [u8; BODY_LEN])) -> String {
        let mut body: [u8; BODY_LEN] = URL_SAFE_NO_PAD
            .decode(payload.strip_prefix(PAYLOAD_PREFIX).unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        mutate(&mut body);

        format!("{PAYLOAD_PREFIX}{}", URL_SAFE_NO_PAD.encode(body))
    }

    #[test]
    fn tampering_with_any_field_breaks_the_signature() {
        let codec = codec(SECRET);
        let payload = codec.encode(group(), InviteGeneration::FIRST);

        for mutate in [0usize, GROUP_LEN + GENERATION_LEN - 1, BODY_LEN - 1] {
            let tampered = resign(&payload, |body| body[mutate] ^= 1);

            assert_eq!(codec.decode(&tampered), Err(InviteDecodeError), "{mutate}");
        }
    }

    #[test]
    fn only_the_canonical_representation_is_accepted() {
        let codec = codec(SECRET);
        let payload = codec.encode(group(), InviteGeneration::FIRST);

        for malformed in [
            String::new(),
            PAYLOAD_PREFIX.to_owned(),
            payload.replacen(PAYLOAD_PREFIX, "gh_", 1),
            payload.replacen(PAYLOAD_PREFIX, "", 1),
            format!("{payload}A"),
            format!("{payload}=="),
            payload[..payload.len() - 1].to_owned(),
            payload.replacen('A', "!", 1),
            "A".repeat(MAX_PAYLOAD_LEN + 1),
        ] {
            assert_eq!(
                codec.decode(&malformed),
                Err(InviteDecodeError),
                "{malformed}"
            );
        }
    }

    #[test]
    fn zero_generation_is_rejected_before_the_database() {
        let codec = codec(SECRET);
        let payload = codec.encode(group(), InviteGeneration::FIRST);
        let zeroed = resign(&payload, |body| {
            body[GROUP_LEN..GROUP_LEN + GENERATION_LEN].fill(0)
        });

        assert_eq!(codec.decode(&zeroed), Err(InviteDecodeError));
    }

    #[test]
    fn secret_must_be_exactly_thirty_two_bytes() {
        assert_eq!(
            InviteSigningSecret::from_base64url("AAEC").unwrap_err(),
            InviteSecretError::Length
        );
        assert_eq!(
            InviteSigningSecret::from_base64url("not base64!").unwrap_err(),
            InviteSecretError::Encoding
        );
        assert!(InviteSigningSecret::from_base64url(SECRET).is_ok());
    }

    #[test]
    fn secret_never_reaches_debug_output() {
        let secret = InviteSigningSecret::from_base64url(SECRET).unwrap();

        assert_eq!(format!("{secret:?}"), "InviteSigningSecret([redacted])");
        assert_eq!(
            format!("{:?}", GroupInviteCodec::new(secret)),
            "GroupInviteCodec"
        );
    }
}
