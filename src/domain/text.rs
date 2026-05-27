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
