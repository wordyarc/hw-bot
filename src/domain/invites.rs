use crate::domain::{DomainError, DomainResult};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InviteGeneration(u32);

impl InviteGeneration {
    pub const FIRST: Self = Self(1);

    pub fn new(value: u32) -> DomainResult<Self> {
        if value == 0 {
            return Err(DomainError::InvalidInviteGeneration { value });
        }

        Ok(Self(value))
    }

    pub fn rotated(self) -> DomainResult<Self> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(DomainError::InviteGenerationExhausted)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_starts_at_one() {
        assert_eq!(InviteGeneration::FIRST.as_u32(), 1);
        assert_eq!(
            InviteGeneration::new(0).unwrap_err(),
            DomainError::InvalidInviteGeneration { value: 0 }
        );
    }

    #[test]
    fn rotation_increments_instead_of_wrapping() {
        assert_eq!(InviteGeneration::FIRST.rotated().unwrap().as_u32(), 2);
        assert_eq!(
            InviteGeneration::new(u32::MAX)
                .unwrap()
                .rotated()
                .unwrap_err(),
            DomainError::InviteGenerationExhausted
        );
    }
}
