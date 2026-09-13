use sqlx::PgPool;

use crate::app::ApplicationContainer;
use crate::health::HealthState;
use crate::invites::GroupInviteCodec;

#[derive(Clone, Debug)]
pub struct AppState {
    pub db_pool: PgPool,
    pub application: ApplicationContainer,
    pub invites: GroupInviteCodec,
    pub health: HealthState,
}

impl AppState {
    pub fn new(
        db_pool: PgPool,
        application: ApplicationContainer,
        invites: GroupInviteCodec,
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
