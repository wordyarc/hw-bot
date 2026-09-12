use sqlx::postgres::PgQueryResult;
use thiserror::Error;

use crate::domain::DomainError;

pub type DbResult<T> = Result<T, DbError>;

const UNIQUE_VIOLATION: &str = "23505";

/// Postgres class 57014: `query_canceled`, в том числе по `statement_timeout`.
const STATEMENT_TIMEOUT: &str = "57014";

/// Условный `update ... where <прежнее состояние>`: 0 затронутых строк означает, что состояние
/// уже изменил конкурент. Какой именно ошибкой это назвать, решает вызывающий - у правок задания
/// это конфликт записи, у проверки сдачи доменное правило.
pub(crate) fn ensure_updated(result: PgQueryResult, lost: DbError) -> DbResult<()> {
    if result.rows_affected() == 0 {
        return Err(lost);
    }

    Ok(())
}

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

    #[error("database is saturated")]
    Overloaded,
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
        if let Some(saturation) = saturation(&error) {
            log_saturation(saturation);
            return Self::Overloaded;
        }

        match unique_violation(&error) {
            Some(mapped) => mapped,
            None => Self::Database(error),
        }
    }
}

#[derive(Clone, Copy)]
enum Saturation {
    PoolAcquire,
    Statement,
}

fn saturation(error: &sqlx::Error) -> Option<Saturation> {
    if matches!(error, sqlx::Error::PoolTimedOut) {
        return Some(Saturation::PoolAcquire);
    }
    if error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some(STATEMENT_TIMEOUT)
    {
        return Some(Saturation::Statement);
    }

    None
}

fn log_saturation(saturation: Saturation) {
    let kind = match saturation {
        Saturation::PoolAcquire => "pool_acquire_timeout",
        Saturation::Statement => "statement_timeout",
    };

    tracing::warn!(kind, "database operation timed out");
}

pub(crate) fn log_timeout(error: &sqlx::Error) {
    if let Some(saturation) = saturation(error) {
        log_saturation(saturation);
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

    #[test]
    fn an_exhausted_pool_is_saturation_and_not_a_database_error() {
        assert!(matches!(
            DbError::from(sqlx::Error::PoolTimedOut),
            DbError::Overloaded
        ));
    }
}
