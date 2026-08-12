use thiserror::Error;

use crate::domain::DomainError;

pub type DbResult<T> = Result<T, DbError>;

/// Postgres class 23505: `unique_violation`.
const UNIQUE_VIOLATION: &str = "23505";

#[derive(Debug, Error)]
pub enum DbError {
    #[error("database error: {0}")]
    Database(sqlx::Error),

    #[error(transparent)]
    RuleViolation(DomainError),

    /// Только для чтения строк: `#[from]` означает, что любой `?` по `DomainError` внутри слоя db
    /// превращается в 500. Для маппинга строк это и нужно (в БД лежит невалидное состояние), но
    /// бизнес-проверку так писать нельзя - она должна возвращать `RuleViolation`.
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

impl From<sqlx::Error> for DbError {
    fn from(error: sqlx::Error) -> Self {
        log_timeout(&error);
        match unique_violation(&error) {
            Some(mapped) => mapped,
            None => Self::Database(error),
        }
    }
}

pub(crate) fn log_timeout(error: &sqlx::Error) {
    if matches!(error, sqlx::Error::PoolTimedOut) {
        tracing::warn!(
            kind = "pool_acquire_timeout",
            "database operation timed out"
        );
    } else if error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("57014")
    {
        tracing::warn!(kind = "statement_timeout", "database operation timed out");
    }
}

fn unique_violation(error: &sqlx::Error) -> Option<DbError> {
    let database_error = error.as_database_error()?;

    if database_error.code().as_deref() != Some(UNIQUE_VIOLATION) {
        return None;
    }

    let rule = |rule| Some(DbError::RuleViolation(rule));

    match database_error.constraint()? {
        "users_telegram_user_id_key" => rule(DomainError::UserAlreadyRegistered),
        "submissions_one_pending_per_student_assignment_idx" => {
            rule(DomainError::SubmissionAlreadyPendingReview)
        }
        "submissions_one_accepted_per_student_assignment_idx" => {
            rule(DomainError::SubmissionAlreadyAccepted)
        }
        // Двойной тап: этот ключ проверяется раньше индекса по pending, поэтому конкурентную
        // вставку ловит он. Соседняя сдача только что вставлена, значит заведомо ждёт проверки.
        "submissions_assignment_id_student_id_attempt_number_key" => {
            rule(DomainError::SubmissionAlreadyPendingReview)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_non_unique_violations_as_database_errors() {
        assert!(matches!(
            DbError::from(sqlx::Error::RowNotFound),
            DbError::Database(_)
        ));
    }
}
