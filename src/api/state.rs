use sqlx::PgPool;

use crate::app::ApplicationContainer;
use crate::health::HealthState;
use crate::invites::CourseInviteCodec;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db_pool: PgPool,
    pub application: ApplicationContainer,
    pub invites: CourseInviteCodec,
    pub health: HealthState,
}

impl AppState {
    pub fn new(
        db_pool: PgPool,
        application: ApplicationContainer,
        invites: CourseInviteCodec,
        health: HealthState,
    ) -> Self {
        Self {
            application,
            db_pool,
            invites,
            health,
        }
    }
}
