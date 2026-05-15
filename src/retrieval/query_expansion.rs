//! Multi-Query Expansion — generates 2–3 reformulations of a query
//! to retrieve from different semantic perspectives.
//!
//! The expansions are template-based (no LLM dependency):
//! 1. Original query (unchanged)
//! 2. Broadening: "X Systems und Konfigurationen"
//! 3. Narrowing: key noun phrases only

/// Expand a query into 2–3 reformulations for multi-perspective retrieval.
pub fn expand_query(query: &str) -> Vec<String> {
    let mut expanded = vec![query.to_string()];

    // Extract key nouns/compounds (German + English)
    let keywords = extract_keywords(query);

    if !keywords.is_empty() {
        // Broadening: general category from first keyword
        if keywords.len() >= 1 {
            let broad = format!(
                "{} Systeme, Tools und Konfigurationen",
                keywords[0]
            );
            if broad != query {
                expanded.push(broad);
            }
        }

        // Narrowing: just the key terms concatenated
        if keywords.len() >= 2 {
            let narrow = keywords.join(" ");
            if narrow != query && !expanded.contains(&narrow) {
                expanded.push(narrow);
            }
        }
    }

    // Deduplicate, max 3
    expanded.dedup();
    expanded.truncate(3);
    expanded
}

/// Extract significant nouns and compound terms from a query.
/// Handles both German and English.
fn extract_keywords(query: &str) -> Vec<String> {
    // Split on word boundaries and keep significant tokens
    let words: Vec<&str> = query
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';' || c == '-' || c == ':')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();

    // Filter out short/stop words
    let stop_words: &[&str] = &[
        "der", "die", "das", "und", "oder", "mit", "von", "für", "auf", "in",
        "the", "a", "an", "is", "of", "to", "for", "with", "and", "or",
        "wie", "was", "warum", "welche", "welcher", "welches",
        "ein", "eine", "einen", "einem",
        "ich", "wir", "unser", "unsere",
    ];

    words
        .into_iter()
        .filter(|w| w.len() > 2)
        .filter(|w| !stop_words.contains(&w.to_lowercase().as_str()))
        .map(|w| w.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_keyword() {
        let result = expand_query("Redis");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "Redis");
        assert!(result[1].contains("Redis"));
    }

    #[test]
    fn test_multi_keyword() {
        let result = expand_query("Redis als Message-Queue");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Redis als Message-Queue"); // original
        assert!(result[1].contains("Redis")); // broadening
        assert!(result[2].contains("Message-Queue") || result[2].contains("Message")); // narrowing
    }

    #[test]
    fn test_dedup() {
        let result = expand_query("Test Test");
        // "Test" → original + "Test Systeme..." → 2
        assert!(result.len() <= 3);
        assert_eq!(result[0], "Test Test");
        assert!(result[1].contains("Test"));
    }

    #[test]
    fn test_empty() {
        let result = expand_query("");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "");
    }

    #[test]
    fn test_stop_word_only() {
        let result = expand_query("der die das");
        assert_eq!(result.len(), 1); // only original
    }
}
