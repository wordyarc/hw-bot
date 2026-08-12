use sqlx::PgPool;

use crate::app::ApplicationContainer;
use crate::health::HealthState;

#[derive(Clone, Debug)]
pub struct AppState {
    pub app_name: String,
    pub db_pool: PgPool,
    pub application: ApplicationContainer,
    pub health: HealthState,
}

impl AppState {
    pub fn new(
        app_name: impl Into<String>,
        db_pool: PgPool,
        application: ApplicationContainer,
        health: HealthState,
    ) -> Self {
        Self {
            app_name: app_name.into(),
            application,
            db_pool,
            health,
        }
    }
}
