pub(crate) const MESSAGE_LIMIT: usize = 4096;

pub(crate) fn utf16_prefix(text: &str, limit: usize) -> &str {
    let mut units = 0;
    let end = text
        .char_indices()
        .find(|(_, character)| {
            units += character.len_utf16();
            units > limit
        })
        .map_or(text.len(), |(index, _)| index);
    &text[..end]
}

const ELLIPSIS: &str = "...";

pub(crate) fn truncate(text: &str, limit: usize) -> String {
    if text.encode_utf16().count() <= limit {
        return text.to_owned();
    }
    let prefix = utf16_prefix(text, limit.saturating_sub(ELLIPSIS.len()));
    format!("{prefix}{ELLIPSIS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_within_the_limit_is_kept_whole() {
        assert_eq!(truncate("🦀Привет", 8), "🦀Привет");
    }

    #[test]
    fn a_truncated_text_fits_the_limit_together_with_the_ellipsis() {
        assert_eq!(truncate("🦀Привет", 7), "🦀Пр...");
        assert_eq!(truncate("ab🦀cd", 5), "ab...");
        for limit in 3..12 {
            let text = truncate("я🦀a".repeat(5).as_str(), limit);
            assert!(text.encode_utf16().count() <= limit, "{limit}: {text}");
            assert!(text.ends_with(ELLIPSIS), "{limit}: {text}");
        }
    }

    #[test]
    fn a_limit_inside_a_surrogate_pair_keeps_the_whole_character_out() {
        assert_eq!(utf16_prefix("a🦀", 2), "a");
        assert_eq!(utf16_prefix("a🦀", 3), "a🦀");
    }
}
