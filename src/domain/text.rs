use crate::domain::{DomainError, DomainResult};

pub const DISPLAY_NAME_MAX_LEN: usize = 100;
pub const STUDENT_GROUP_CODE_MAX_LEN: usize = 64;
pub const SUBJECT_NAME_MAX_LEN: usize = 200;
pub const ASSIGNMENT_TITLE_MAX_LEN: usize = 200;
pub const USER_TEXT_MAX_LEN: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonEmptyString(String);

impl NonEmptyString {
    pub fn new(field: &'static str, value: impl Into<String>) -> DomainResult<Self> {
        let mut value = value.into();
        value.truncate(value.trim_end().len());
        value.replace_range(..value.len() - value.trim_start().len(), "");

        if value.is_empty() {
            return Err(DomainError::EmptyField { field });
        }
        if value.contains('\0') {
            return Err(DomainError::NullCharacter { field });
        }

        Ok(Self(value))
    }

    pub fn new_bounded(
        field: &'static str,
        value: impl Into<String>,
        max_len: usize,
    ) -> DomainResult<Self> {
        let value = Self::new(field, value)?;
        let actual = value.0.chars().count();

        if actual > max_len {
            return Err(DomainError::FieldTooLong {
                field,
                max: max_len,
                actual,
            });
        }

        Ok(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_value() {
        let value = NonEmptyString::new("field", "  text  ").unwrap();

        assert_eq!(value.as_str(), "text");
    }

    #[test]
    fn rejects_null_characters_before_persistence() {
        assert_eq!(
            NonEmptyString::new("content", "pull/1\0").unwrap_err(),
            DomainError::NullCharacter { field: "content" },
        );
    }

    #[test]
    fn rejects_empty_value_after_trim() {
        assert_eq!(
            NonEmptyString::new("field", "   ").unwrap_err(),
            DomainError::EmptyField { field: "field" }
        );
    }

    #[test]
    fn returns_owned_string() {
        let value = NonEmptyString::new("field", "text").unwrap();

        assert_eq!(value.into_string(), "text");
    }

    #[test]
    fn bounded_value_counts_unicode_scalars() {
        assert!(NonEmptyString::new_bounded("field", "ёж", 2).is_ok());
        assert_eq!(
            NonEmptyString::new_bounded("field", "ёжик", 3).unwrap_err(),
            DomainError::FieldTooLong {
                field: "field",
                max: 3,
                actual: 4,
            }
        );
    }
}
