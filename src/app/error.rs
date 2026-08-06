use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::db::DbError;
use crate::domain::DomainError;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    Domain(DomainError),
    NotFound { resource: &'static str },
    Conflict { message: &'static str },
    Db(DbError),
}

impl Display for AppError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain(error) => write!(formatter, "{error}"),
            Self::NotFound { resource } => write!(formatter, "{resource} not found"),
            Self::Conflict { message } => write!(formatter, "{message}"),
            Self::Db(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Domain(error) => Some(error),
            Self::Db(error) => Some(error),
            Self::NotFound { .. } | Self::Conflict { .. } => None,
        }
    }
}

impl From<DomainError> for AppError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Conflict { message } => Self::Conflict { message },
            other => Self::Db(other),
        }
    }
}
