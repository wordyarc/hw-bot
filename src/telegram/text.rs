pub(crate) const MESSAGE_LIMIT: usize = 4096;

pub(crate) fn truncate(text: &str, limit: usize, ellipsis: bool) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut result: String = text.chars().take(limit).collect();
    if ellipsis {
        result.push_str("...");
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_counts_unicode_characters_and_appends_an_optional_ellipsis() {
        assert_eq!(truncate("🦀Привет", 5, true), "🦀Прив...");
        assert_eq!(truncate("🦀Привет", 5, false), "🦀Прив");
        assert_eq!(truncate("🦀Привет", 7, true), "🦀Привет");
        assert_eq!(truncate("🦀Привет", 0, false), "");
    }
}
