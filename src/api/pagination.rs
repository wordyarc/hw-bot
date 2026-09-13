use super::ApiError;
use crate::pagination::{Page, PageRequest};
use axum::extract::{FromRequestParts, Query};
use axum::http::{HeaderMap, StatusCode, request::Parts};

pub struct Pagination(pub PageRequest);

impl<S: Send + Sync> FromRequestParts<S> for Pagination {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<PageRequest>::from_request_parts(parts, state)
            .await
            .map(|Query(page)| Self(page))
            .map_err(|_| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_query",
                    "page must be an unsigned 32-bit integer",
                )
            })
    }
}

pub fn page_headers<T>(page: &Page<T>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-page",
        page.page.to_string().parse().expect("numeric header"),
    );
    if let Some(next) = page.next_page {
        headers.insert(
            "x-next-page",
            next.to_string().parse().expect("numeric header"),
        );
    }
    headers
}
