use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};

use crate::api::ApiError;
use crate::domain::TelegramUserId;

pub const ACTOR_HEADER: &str = "x-telegram-user-id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Actor(pub TelegramUserId);

impl Actor {
    fn from_headers(headers: &HeaderMap) -> Result<Self, ApiError> {
        let header = headers.get(ACTOR_HEADER).ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                format!("{ACTOR_HEADER} header is missing"),
            )
        })?;

        header
            .to_str()
            .ok()
            .and_then(|raw| raw.parse().ok())
            .and_then(|id| TelegramUserId::new(id).ok())
            .map(Self)
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_actor",
                    format!("{ACTOR_HEADER} header is not a telegram user id"),
                )
            })
    }
}

impl<S> FromRequestParts<S> for Actor
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Self::from_headers(&parts.headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract(header: Option<&str>) -> Result<Actor, ApiError> {
        let mut headers = HeaderMap::new();

        if let Some(value) = header {
            headers.insert(ACTOR_HEADER, value.parse().expect("valid header value"));
        }

        Actor::from_headers(&headers)
    }

    #[test]
    fn reads_the_telegram_user_id_from_the_header() {
        let Actor(actor) = extract(Some("1001")).expect("an actor");

        assert_eq!(actor.as_i64(), 1001);
    }

    #[test]
    fn a_missing_header_is_unauthenticated() {
        let error = extract(None).expect_err("an error");

        assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(error.code(), "unauthenticated");
    }

    #[test]
    fn a_non_numeric_header_is_a_bad_request() {
        let error = extract(Some("ivan")).expect_err("an error");

        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(error.code(), "invalid_actor");
    }

    #[test]
    fn a_non_positive_header_is_a_bad_request() {
        for raw in ["0", "-1"] {
            let error = extract(Some(raw)).expect_err("an error");

            assert_eq!(error.code(), "invalid_actor", "header {raw}");
        }
    }
}
