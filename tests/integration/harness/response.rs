use axum::http::StatusCode;
use serde_json::{Value, json};

/// Ответ вместе с телом. Смысл в том, что тело переживает провалившийся ассерт по статусу:
/// раньше неожиданный 500 выглядел как голый `left: 500, right: 403` без единого намёка на причину.
pub struct TestResponse {
    pub status: StatusCode,
    pub body: Value,
}

impl TestResponse {
    #[track_caller]
    pub fn expect(self, status: StatusCode) -> Value {
        assert_eq!(
            self.status, status,
            "unexpected status\nbody: {:#}",
            self.body
        );

        self.body
    }

    #[track_caller]
    pub fn ok(self) -> Value {
        self.expect(StatusCode::OK)
    }

    #[track_caller]
    pub fn created(self) -> Value {
        self.expect(StatusCode::CREATED)
    }

    #[track_caller]
    pub fn no_content(self) {
        let body = self.expect(StatusCode::NO_CONTENT);

        assert_eq!(body, Value::Null, "204 must not carry a body");
    }

    #[track_caller]
    pub fn error(self, status: StatusCode, code: &str) -> Value {
        let body = self.expect(status);

        assert_eq!(
            body["error"]["code"],
            json!(code),
            "unexpected error code\nbody: {body:#}"
        );

        body["error"].clone()
    }

    #[track_caller]
    pub fn not_found(self, message: &str) {
        let error = self.error(StatusCode::NOT_FOUND, "not_found");

        assert_eq!(error["message"], json!(message));
    }
}
