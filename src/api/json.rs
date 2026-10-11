use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};

use crate::api::ApiError;

#[derive(Debug)]
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(request, state).await?;

        Ok(Self(value))
    }
}
