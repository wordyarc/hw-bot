use thiserror::Error;

use crate::domain::DomainError;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    RuleViolation(DomainError),

    #[error("invalid domain state from database: {0}")]
    CorruptedState(#[from] DomainError),

    #[error("invalid database value for {field}: {value}")]
    InvalidDatabaseValue { field: &'static str, value: String },

    #[error("{message}")]
    Conflict { message: &'static str },
}

impl DbError {
    pub(crate) fn invalid_value(field: &'static str, value: impl Into<String>) -> Self {
        Self::InvalidDatabaseValue {
            field,
            value: value.into(),
        }
    }
}
