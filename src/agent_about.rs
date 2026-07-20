//! Bounded agent descriptions for context and operator surfaces.

pub(crate) const AGENT_ABOUT_MAX_CHARS: usize = 200;

/// Compact and bound an agent description before injecting it into context.
///
/// Native profile descriptions sometimes contain escaped newlines and long
/// usage examples. Those remain intact in the source profile; agent-facing
/// roster views need only a concise routing hint.
pub(crate) fn for_injection(value: &str) -> String {
    compact(value, AGENT_ABOUT_MAX_CHARS)
}

/// Normalize an agent description and bound it to a caller-owned display size.
pub(crate) fn compact(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let compact = value
        .replace("\\r\\n", " ")
        .replace("\\n", " ")
        .replace("\\r", " ")
        .replace("\\t", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }

    let prefix = compact.chars().take(max_chars - 1).collect::<String>();
    let boundary = prefix
        .rfind(' ')
        .filter(|index| prefix[..*index].chars().count() >= max_chars / 2);
    let kept = boundary.map_or(prefix.trim_end(), |index| &prefix[..index]);
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_whitespace_and_escaped_newlines() {
        assert_eq!(
            for_injection("  Routes\\n review\nwork\t carefully  "),
            "Routes review work carefully"
        );
    }

    #[test]
    fn preserves_descriptions_at_the_limit() {
        let exact = "x".repeat(AGENT_ABOUT_MAX_CHARS);
        assert_eq!(for_injection(&exact), exact);
    }

    #[test]
    fn truncates_unicode_safely_at_the_character_limit() {
        let result = for_injection(&"é".repeat(AGENT_ABOUT_MAX_CHARS + 10));
        assert_eq!(result.chars().count(), AGENT_ABOUT_MAX_CHARS);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn prefers_a_word_boundary() {
        let result = for_injection(&"routing description ".repeat(20));
        assert!(result.chars().count() <= AGENT_ABOUT_MAX_CHARS);
        let stem = result.strip_suffix('…').unwrap();
        assert!(stem.ends_with("routing") || stem.ends_with("description"));
    }
}
