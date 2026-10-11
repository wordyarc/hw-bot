use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use thiserror::Error;

use crate::domain::{InviteGeneration, TeachingCourseId};

/// В payload команды `/start` Telegram допускает не больше 64 символов.
pub const MAX_PAYLOAD_LEN: usize = 64;

const PAYLOAD_PREFIX: &str = "ci_";
const DOMAIN_SEPARATION: &[u8] = b"hw-bot/course-invite/v1";
const SECRET_LEN: usize = 32;
const TAG_LEN: usize = 16;
const COURSE_LEN: usize = 16;
const GENERATION_LEN: usize = 4;
const BODY_LEN: usize = COURSE_LEN + GENERATION_LEN + TAG_LEN;
/// Фиксированная длина полей позволяет читать их по смещениям без разделителя.
/// Для разделителя нет свободного символа: `-` и `_` уже заняты в base64url,
/// а расширить алфавит payload команды `/start` Telegram не разрешает.
const ENCODED_BODY_LEN: usize = BODY_LEN / 3 * 4;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedInvite {
    course_id: TeachingCourseId,
    generation: InviteGeneration,
}

impl VerifiedInvite {
    pub fn course_id(self) -> TeachingCourseId {
        self.course_id
    }

    pub fn generation(self) -> InviteGeneration {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
#[error("course invite is not valid")]
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
pub struct CourseInviteCodec {
    secret: InviteSigningSecret,
}

impl CourseInviteCodec {
    pub fn new(secret: InviteSigningSecret) -> Self {
        Self { secret }
    }

    pub fn encode(&self, course_id: TeachingCourseId, generation: InviteGeneration) -> String {
        let mut body = [0u8; BODY_LEN];
        body[..COURSE_LEN].copy_from_slice(course_id.as_uuid().as_bytes());
        body[COURSE_LEN..COURSE_LEN + GENERATION_LEN]
            .copy_from_slice(&generation.as_u32().to_be_bytes());
        body[COURSE_LEN + GENERATION_LEN..]
            .copy_from_slice(&self.tag(course_id, generation)[..TAG_LEN]);

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
        let (course, rest) = body.split_at(COURSE_LEN);
        let (generation, tag) = rest.split_at(GENERATION_LEN);

        let course_id =
            TeachingCourseId::from_uuid(uuid::Uuid::from_slice(course).expect("16 bytes"));
        let generation =
            InviteGeneration::new(u32::from_be_bytes(generation.try_into().expect("4 bytes")))
                .map_err(|_| InviteDecodeError)?;

        self.mac(course_id, generation)
            .verify_truncated_left(tag)
            .map_err(|_| InviteDecodeError)?;

        Ok(VerifiedInvite {
            course_id,
            generation,
        })
    }

    fn mac(&self, course_id: TeachingCourseId, generation: InviteGeneration) -> HmacSha256 {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret.0).expect("HMAC accepts a key of any length");
        mac.update(DOMAIN_SEPARATION);
        mac.update(course_id.as_uuid().as_bytes());
        mac.update(&generation.as_u32().to_be_bytes());
        mac
    }

    fn tag(&self, course_id: TeachingCourseId, generation: InviteGeneration) -> [u8; 32] {
        self.mac(course_id, generation)
            .finalize()
            .into_bytes()
            .into()
    }
}

impl fmt::Debug for CourseInviteCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CourseInviteCodec")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    const OTHER_SECRET: &str = "f39_f39_f39_f39_f39_f39_f39_f39_f39_f39_f38";
    const COURSE: uuid::Uuid = uuid::uuid!("2f4b7c1e-0d9a-4a6b-8c3d-5e6f70819293");

    fn codec(secret: &str) -> CourseInviteCodec {
        CourseInviteCodec::new(InviteSigningSecret::from_base64url(secret).unwrap())
    }

    fn course() -> TeachingCourseId {
        TeachingCourseId::from_uuid(COURSE)
    }

    #[test]
    fn payload_round_trips_through_the_codec() {
        let codec = codec(SECRET);
        let generation = InviteGeneration::new(7).unwrap();

        let decoded = codec.decode(&codec.encode(course(), generation)).unwrap();

        assert_eq!(decoded.course_id(), course());
        assert_eq!(decoded.generation(), generation);
    }

    #[test]
    fn payload_fits_the_telegram_start_limit() {
        let payload = codec(SECRET).encode(course(), InviteGeneration::new(u32::MAX).unwrap());

        assert_eq!(payload.len(), 51, "{payload}");
        assert!(payload.len() <= MAX_PAYLOAD_LEN, "{payload}");
    }

    /// Новый релиз должен читать прежний wire-формат, иначе выданные ссылки сломаются.
    #[test]
    fn wire_format_stays_compatible_between_versions() {
        assert_eq!(
            codec(SECRET).encode(course(), InviteGeneration::FIRST),
            "ci_L0t8Hg2aSmuMPV5vcIGSkwAAAAF9G1Lh9tZyOiqyQ1Z7HOFR"
        );
    }

    #[test]
    fn a_payload_of_the_former_group_link_format_is_rejected() {
        let codec = codec(SECRET);
        let payload = codec.encode(course(), InviteGeneration::FIRST);

        for former in [
            "gi_L0t8Hg2aSmuMPV5vcIGSkwAAAAEdut_Q-iaKZ9reQaSSVLAI".to_owned(),
            payload.replacen(PAYLOAD_PREFIX, "gi_", 1),
        ] {
            assert_eq!(codec.decode(&former), Err(InviteDecodeError), "{former}");
        }
    }

    #[test]
    fn a_different_secret_does_not_verify_the_payload() {
        let payload = codec(SECRET).encode(course(), InviteGeneration::FIRST);

        assert_eq!(codec(OTHER_SECRET).decode(&payload), Err(InviteDecodeError));
    }

    /// Побайтовая правка готового payload проверяет подпись. Повторный вызов `encode`
    /// вместо такой правки проверял бы только работу кодека.
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
        let payload = codec.encode(course(), InviteGeneration::FIRST);

        for mutate in [0usize, COURSE_LEN + GENERATION_LEN - 1, BODY_LEN - 1] {
            let tampered = resign(&payload, |body| body[mutate] ^= 1);

            assert_eq!(codec.decode(&tampered), Err(InviteDecodeError), "{mutate}");
        }
    }

    #[test]
    fn only_the_canonical_representation_is_accepted() {
        let codec = codec(SECRET);
        let payload = codec.encode(course(), InviteGeneration::FIRST);

        for malformed in [
            String::new(),
            PAYLOAD_PREFIX.to_owned(),
            payload.replacen(PAYLOAD_PREFIX, "cj_", 1),
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
        let payload = codec.encode(course(), InviteGeneration::FIRST);
        let zeroed = resign(&payload, |body| {
            body[COURSE_LEN..COURSE_LEN + GENERATION_LEN].fill(0)
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
            format!("{:?}", CourseInviteCodec::new(secret)),
            "CourseInviteCodec"
        );
    }
}
