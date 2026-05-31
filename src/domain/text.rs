use crate::domain::{DomainError, DomainResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonEmptyString(String);

impl NonEmptyString {
    pub fn new(field: &'static str, value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into().trim().to_owned();

        if value.is_empty() {
            return Err(DomainError::EmptyField { field });
        }

        Ok(Self(value))
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
}
