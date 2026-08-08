use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use serde_json::{Value, json};

/// Ответ вместе с телом. Смысл в том, что тело переживает провалившийся ассерт по статусу:
/// раньше неожиданный 500 выглядел как голый `left: 500, right: 403` без единого намёка на причину.
pub struct TestResponse {
    pub status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl TestResponse {
    pub fn new(status: StatusCode, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    #[track_caller]
    pub fn expect(self, status: StatusCode) -> Self {
        assert_eq!(
            self.status,
            status,
            "unexpected status\nbody: {}",
            String::from_utf8_lossy(&self.body)
        );

        self
    }

    #[track_caller]
    fn expect_json(self, status: StatusCode) -> Value {
        let response = self.expect(status);

        serde_json::from_slice(&response.body).unwrap_or_else(|error| {
            panic!(
                "response body is not json: {error}\nbody: {}",
                String::from_utf8_lossy(&response.body)
            )
        })
    }

    #[track_caller]
    pub fn ok(self) -> Value {
        self.expect_json(StatusCode::OK)
    }

    #[track_caller]
    pub fn created(self) -> Value {
        self.expect_json(StatusCode::CREATED)
    }

    #[track_caller]
    pub fn no_content(self) {
        let response = self.expect(StatusCode::NO_CONTENT);

        assert!(response.body.is_empty(), "204 must not carry a body");
    }

    #[track_caller]
    pub fn error(self, status: StatusCode, code: &str) -> Value {
        let body = self.expect_json(status);

        assert_eq!(
            body["error"]["code"],
            json!(code),
            "unexpected error code\nbody: {body:#}"
        );

        body["error"].clone()
    }

    #[track_caller]
    pub fn not_found(self) {
        self.error(StatusCode::NOT_FOUND, "not_found");
    }
}
