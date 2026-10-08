use sqlx::postgres::PgQueryResult;
use thiserror::Error;

use crate::domain::DomainError;

pub type DbResult<T> = Result<T, DbError>;

const UNIQUE_VIOLATION: &str = "23505";

/// Код PostgreSQL 57014 (`query_canceled`) приходит и при срабатывании `statement_timeout`.
const STATEMENT_TIMEOUT: &str = "57014";

/// Если `update ... where <прежнее состояние>` не затронул строк, конкурент успел изменить запись.
/// Ошибку выбирает вызывающий: правка задания возвращает конфликт, проверка сдачи - доменный отказ.
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

    /// При восстановлении строки `#[from]` переводит `DomainError` через `?` в 500:
    /// нарушение инварианта в сохранённых данных считается повреждением БД.
    /// Для бизнес-отказов нужно явно возвращать `RuleViolation`, иначе они тоже станут 500.
    #[error("invalid domain state from database: {0}")]
    CorruptedState(#[from] DomainError),

    #[error("invalid database value for {field}: {value}")]
    InvalidDatabaseValue { field: &'static str, value: String },

    /// Условный update не затронул строк: запись успел изменить конкурент.
    #[error("{message}")]
    Conflict { message: &'static str },

    /// Вызывающий нарушил контракт функции. Это ошибка программы, а не гонка, поэтому
    /// `on_conflict` её не перехватывает и наружу она уходит как внутренняя.
    #[error("{message}")]
    Precondition { message: &'static str },

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
        "submissions_one_accepted_per_student_assignment_kind_idx" => {
            rule(DomainError::SubmissionAlreadyAccepted)
        }
        // При двойном тапе первым срабатывает этот ключ, не индекс по pending.
        // Конкурирующая сдача только что создана и ещё находится на проверке.
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
