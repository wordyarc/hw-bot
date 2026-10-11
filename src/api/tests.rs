use axum::body::{Body, Bytes, to_bytes};
use axum::http::Request;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::*;
use crate::app::{AppError, ApplicationContainer, FilePolicy};
use crate::domain::AssignmentId;
use crate::health::HealthState;
use crate::invites::{GroupInviteCodec, InviteSigningSecret};
use crate::test_support::{closed_pool, t0};

async fn database_free_router() -> Router {
    let pool = closed_pool().await;
    let application = ApplicationContainer::new(
        pool.clone(),
        FilePolicy {
            max_file_size_bytes: 1,
            max_files_per_assignment: 1,
            max_files_per_submission: 1,
        },
    );
    let invites = GroupInviteCodec::new(
        InviteSigningSecret::from_base64url("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8").unwrap(),
    );
    router(
        AppState::new(pool, application, invites, HealthState::new(false, false)),
        Duration::from_secs(1),
    )
}

async fn assert_error(response: Response, expected: StatusCode, code: &str) -> Value {
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        status,
        expected,
        "body: {}",
        String::from_utf8_lossy(&bytes)
    );
    let body: Value = serde_json::from_slice(&bytes).expect("JSON error envelope");
    assert_eq!(body["error"]["code"], code, "{body}");
    body
}

fn json_request(path: &str, body: impl Into<Body>) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .body(body.into())
        .unwrap()
}

async fn rejects_json<T: DeserializeOwned + Send + 'static>(body: Value) {
    let router = Router::new().route("/", post(|JsonBody(_): JsonBody<T>| async {}));
    let response = router
        .oneshot(json_request("/", serde_json::to_vec(&body).unwrap()))
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::UNPROCESSABLE_ENTITY,
        "invalid_request_body",
    )
    .await;
}

#[tokio::test]
async fn reads_commands_and_registration_require_the_identity_header() {
    let router = database_free_router().await;
    for request in [
        Request::get("/me").body(Body::empty()).unwrap(),
        json_request(
            &format!("/assignments/{}/submissions", AssignmentId::new().as_uuid()),
            "{}",
        ),
        json_request("/students", "{}"),
    ] {
        let response = router.clone().oneshot(request).await.unwrap();
        assert_error(response, StatusCode::UNAUTHORIZED, "unauthenticated").await;
    }
}

#[tokio::test]
async fn an_invalid_actor_header_uses_the_error_envelope_before_database_access() {
    let request = Request::get("/me/assignments")
        .header(ACTOR_HEADER, "ivan")
        .body(Body::empty())
        .unwrap();
    let response = database_free_router().await.oneshot(request).await.unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "invalid_actor").await;
}

#[tokio::test]
async fn missing_routes_including_the_removed_group_directory_use_the_error_envelope() {
    let router = database_free_router().await;
    for path in ["/does-not-exist", "/student-groups"] {
        let response = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_error(response, StatusCode::NOT_FOUND, "not_found").await;
    }
}

#[tokio::test]
async fn unsupported_methods_use_the_error_envelope() {
    let response = database_free_router()
        .await
        .oneshot(Request::post("/health/live").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
    )
    .await;
}

#[tokio::test]
async fn liveness_survives_an_unavailable_database_but_readiness_does_not() {
    let router = database_free_router().await;
    for (path, expected) in [
        ("/health/live", StatusCode::OK),
        ("/health/ready", StatusCode::SERVICE_UNAVAILABLE),
    ] {
        let response = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{path}");
    }
}

#[tokio::test]
async fn malformed_typed_path_uses_the_error_envelope() {
    let router = Router::new().route("/{id}", get(|ApiPath(_): ApiPath<AssignmentId>| async {}));
    let response = router
        .oneshot(Request::get("/not-a-uuid").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_error(response, StatusCode::BAD_REQUEST, "invalid_path").await;
}

#[tokio::test]
async fn invalid_pagination_uses_the_error_envelope() {
    let router = Router::new().route(
        "/",
        get(|pagination::Pagination(_): pagination::Pagination| async {}),
    );
    for query in ["page=-1", "page=no", "page=4294967296", "unknown=1"] {
        let response = router
            .clone()
            .oneshot(
                Request::get(format!("/?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_error(response, StatusCode::BAD_REQUEST, "invalid_query").await;
    }
}

#[tokio::test]
async fn the_actor_is_not_accepted_in_the_submission_body() {
    rejects_json::<submissions::SubmitHomeworkRequest>(
        json!({"content": "pull/1", "telegram_user_id": 123}),
    )
    .await;
}

#[tokio::test]
async fn student_is_not_a_staff_registration_role() {
    rejects_json::<users::RegisterStaffRequest>(
        json!({"telegram_user_id": 123, "display_name": "Student", "role": "student"}),
    )
    .await;
}

#[tokio::test]
async fn unknown_review_decision_is_rejected() {
    rejects_json::<review::ReviewSubmissionRequest>(
        json!({"decision": "maybe", "comment": "Undecided"}),
    )
    .await;
}

#[tokio::test]
async fn the_removed_closing_date_is_rejected_in_every_assignment_request_including_null() {
    for value in [json!(t0()), Value::Null] {
        rejects_json::<assignments::CreateAssignmentRequest>(json!({
            "title": "Task", "body": "Body", "max_attempts": 2,
            "deadline_at": t0(), "late_closes_at": value,
        }))
        .await;
        rejects_json::<assignments::ChangeDeadlineRequest>(json!({
            "deadline_at": t0(), "late_closes_at": value,
        }))
        .await;
        rejects_json::<assignments::ChangeGradingRequest>(json!({
            "max_score": 5, "late_closes_at": value,
        }))
        .await;
    }
}

#[tokio::test]
async fn normalization_preserves_the_file_size_error() {
    let error = AppError::FileTooLarge {
        max_bytes: 5,
        actual_bytes: 6,
    };
    let message = error.to_string();
    let response = normalize_transport_error(ApiError::from(error).into_response()).await;
    let body = assert_error(response, StatusCode::PAYLOAD_TOO_LARGE, "file_too_large").await;
    assert_eq!(body["error"]["message"], message);
}

fn limited_json_router() -> Router {
    Router::new()
        .route(
            "/",
            post(|JsonBody(_): JsonBody<users::RegisterStudentRequest>| async {}),
        )
        .layer(RequestBodyLimitLayer::new(REQUEST_BODY_LIMIT))
        .layer(axum::middleware::map_response(normalize_transport_error))
}

#[tokio::test]
async fn a_body_at_the_transport_limit_reaches_the_handler() {
    let mut body =
        serde_json::to_vec(&json!({"display_name": "Student", "invite_token": "token"})).unwrap();
    body.resize(REQUEST_BODY_LIMIT, b' ');
    let response = limited_json_router()
        .oneshot(json_request("/", body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_real_router_rejects_an_oversized_registration_body_before_database_access() {
    let mut request = json_request("/students", vec![b' '; REQUEST_BODY_LIMIT + 1]);
    request
        .headers_mut()
        .insert(ACTOR_HEADER, "123".parse().unwrap());
    let response = database_free_router().await.oneshot(request).await.unwrap();
    assert_error(
        response,
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_body_too_large",
    )
    .await;
}

#[tokio::test]
async fn streamed_body_over_the_limit_keeps_the_transport_error_code() {
    let chunks = futures::stream::iter([
        Ok::<_, std::io::Error>(Bytes::from_static(b"{\"display_name\":\"")),
        Ok(Bytes::from(vec![b'a'; REQUEST_BODY_LIMIT])),
        Ok(Bytes::from_static(b"\",\"invite_token\":\"token\"}")),
    ]);
    let response = limited_json_router()
        .oneshot(json_request("/", Body::from_stream(chunks)))
        .await
        .unwrap();
    assert_error(
        response,
        StatusCode::PAYLOAD_TOO_LARGE,
        "request_body_too_large",
    )
    .await;
}

#[tokio::test]
async fn a_request_timeout_is_logged_with_its_endpoint() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Writer(Arc<Mutex<Vec<u8>>>);
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buffer = Writer(Arc::default());
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_env_filter("hw_bot=info,tower_http=info")
        .with_writer(move || writer.clone())
        .finish();
    let _logs = tracing::subscriber::set_default(subscriber);
    let router = with_transport_layers(
        Router::new().route(
            "/slow",
            get(|| async { tokio::time::sleep(Duration::from_secs(5)).await }),
        ),
        Duration::from_millis(10),
    );

    let response = router
        .oneshot(Request::get("/slow").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_error(response, StatusCode::REQUEST_TIMEOUT, "request_timeout").await;
    let bytes = buffer.0.lock().unwrap();
    let event = serde_json::Deserializer::from_slice(&bytes)
        .into_iter::<Value>()
        .map(Result::unwrap)
        .find(|event| event["fields"]["kind"] == "request_timeout")
        .expect("the timeout is logged");
    assert_eq!(event["span"]["method"], "GET", "{event}");
    assert_eq!(event["span"]["uri"], "/slow", "{event}");
}
