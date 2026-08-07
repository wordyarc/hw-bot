use std::fmt::Display;

use crate::db::{DbError, DbResult};

pub(crate) fn num<T, U>(field: &'static str, value: T) -> DbResult<U>
where
    T: Copy + Display,
    U: TryFrom<T>,
{
    U::try_from(value).map_err(|_| DbError::invalid_value(field, value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_smallint_overflow_before_insert() {
        assert!(num::<_, i16>("field", i16::MAX as u16).is_ok());
        assert!(matches!(
            num::<_, i16>("field", i16::MAX as u16 + 1),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }

    #[test]
    fn rejects_negative_smallint_from_database() {
        assert!(matches!(
            num::<_, u16>("field", -1i16),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_bigint_from_database() {
        assert!(matches!(
            num::<_, u16>("field", i64::from(u16::MAX) + 1),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }
}
