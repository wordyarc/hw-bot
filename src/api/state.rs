use sqlx::PgPool;

use crate::app::{ReviewService, SubmissionService};

#[derive(Clone, Debug)]
pub struct AppState {
    pub app_name: String,
    pub db_pool: PgPool,
    pub submissions: SubmissionService,
    pub reviews: ReviewService,
}

impl AppState {
    pub fn new(app_name: impl Into<String>, db_pool: PgPool) -> Self {
        Self {
            app_name: app_name.into(),
            submissions: SubmissionService::new(db_pool.clone()),
            reviews: ReviewService::new(db_pool.clone()),
            db_pool,
        }
    }
}
