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

pub(crate) fn truncate(text: &str, limit: usize, ellipsis: bool) -> String {
    let prefix = utf16_prefix(text, limit);
    if prefix.len() == text.len() {
        return text.to_owned();
    }
    let mut result = prefix.to_owned();
    if ellipsis {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_utf16_units_and_appends_an_optional_ellipsis() {
        assert_eq!(truncate("🦀Привет", 5, true), "🦀При...");
        assert_eq!(truncate("🦀Привет", 5, false), "🦀При");
        assert_eq!(truncate("🦀Привет", 8, true), "🦀Привет");
        assert_eq!(truncate("🦀Привет", 0, false), "");
    }

    #[test]
    fn a_limit_inside_a_surrogate_pair_keeps_the_whole_character_out() {
        assert_eq!(utf16_prefix("a🦀", 2), "a");
        assert_eq!(utf16_prefix("a🦀", 3), "a🦀");
    }
}
