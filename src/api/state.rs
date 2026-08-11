use sqlx::PgPool;

use crate::app::ApplicationContainer;

#[derive(Clone, Debug)]
pub struct AppState {
    pub app_name: String,
    pub db_pool: PgPool,
    pub application: ApplicationContainer,
}

impl AppState {
    pub fn new(
        app_name: impl Into<String>,
        db_pool: PgPool,
        application: ApplicationContainer,
    ) -> Self {
        Self {
            app_name: app_name.into(),
            application,
            db_pool,
        }
    }
}
