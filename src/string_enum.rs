macro_rules! str_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident => $value:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            pub fn parse_str(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;

                Self::parse_str(&value).ok_or_else(|| {
                    serde::de::Error::unknown_variant(&value, &[$($value),+])
                })
            }
        }
    };
}

pub(crate) use str_enum;

#[cfg(test)]
mod tests {
    use crate::app::ErrorCode;
    use crate::db::notifications::ReviewOutcome;
    use crate::domain::{AssignmentStatus, EducationLevel, SubmissionStatus};

    macro_rules! assert_round_trips {
        ($($type:ty),+ $(,)?) => {$(
            for variant in <$type>::ALL {
                assert_eq!(<$type>::parse_str(variant.as_str()), Some(*variant));
            }

            assert_eq!(<$type>::parse_str("no_such_variant"), None);
        )+};
    }

    #[test]
    fn every_variant_round_trips_through_its_string_name() {
        assert_round_trips!(
            SubmissionStatus,
            AssignmentStatus,
            EducationLevel,
            ErrorCode,
            ReviewOutcome
        );
    }

    #[test]
    fn serde_reuses_the_same_names() {
        assert_eq!(
            serde_json::to_string(&SubmissionStatus::PendingReview).unwrap(),
            r#""pending_review""#
        );
        assert_eq!(
            serde_json::from_str::<AssignmentStatus>(r#""archived""#).unwrap(),
            AssignmentStatus::Archived
        );
        assert!(serde_json::from_str::<SubmissionStatus>(r#""nope""#).is_err());
    }
}
