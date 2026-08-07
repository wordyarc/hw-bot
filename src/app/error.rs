use thiserror::Error;

use crate::db::DbError;
use crate::domain::DomainError;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error("{resource} not found")]
    NotFound { resource: &'static str },

    #[error("{message}")]
    Conflict { message: &'static str },

    #[error(transparent)]
    Db(DbError),
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::RuleViolation(domain_error) => Self::Domain(domain_error),
            DbError::Conflict { message } => Self::Conflict { message },
            other => Self::Db(other),
        }
    }
}
