use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::domain::DomainError;

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug)]
pub enum DbError {
    Database(sqlx::Error),
    Domain(DomainError),
    InvalidDatabaseValue { field: &'static str, value: String },
    Conflict { message: &'static str },
}

impl Display for DbError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database(error) => write!(formatter, "database error: {error}"),
            Self::Domain(error) => write!(formatter, "invalid domain state from database: {error}"),
            Self::InvalidDatabaseValue { field, value } => {
                write!(formatter, "invalid database value for {field}: {value}")
            }
            Self::Conflict { message } => write!(formatter, "{message}"),
        }
    }
}

impl Error for DbError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Domain(error) => Some(error),
            Self::InvalidDatabaseValue { .. } | Self::Conflict { .. } => None,
        }
    }
}

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl From<DomainError> for DbError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}
