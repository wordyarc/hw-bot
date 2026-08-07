use crate::db::{DbError, DbResult};

pub(crate) fn to_db_smallint(field: &'static str, value: u16) -> DbResult<i16> {
    i16::try_from(value).map_err(|_| DbError::invalid_value(field, value.to_string()))
}

pub(crate) fn try_u16_from_i16(field: &'static str, value: i16) -> DbResult<u16> {
    u16::try_from(value).map_err(|_| DbError::invalid_value(field, value.to_string()))
}

pub(crate) fn try_u16_from_i64(field: &'static str, value: i64) -> DbResult<u16> {
    u16::try_from(value).map_err(|_| DbError::invalid_value(field, value.to_string()))
}

pub(crate) fn try_u8_from_i16(field: &'static str, value: i16) -> DbResult<u8> {
    u8::try_from(value).map_err(|_| DbError::invalid_value(field, value.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_smallint_overflow_before_insert() {
        assert!(to_db_smallint("field", i16::MAX as u16).is_ok());
        assert!(matches!(
            to_db_smallint("field", i16::MAX as u16 + 1),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }

    #[test]
    fn rejects_negative_smallint_from_database() {
        assert!(matches!(
            try_u16_from_i16("field", -1),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_bigint_from_database() {
        assert!(matches!(
            try_u16_from_i64("field", i64::from(u16::MAX) + 1),
            Err(DbError::InvalidDatabaseValue { field: "field", .. })
        ));
    }
}
