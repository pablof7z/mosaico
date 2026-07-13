//! Secret-display helper for the provider configuration prompts.

/// Mask a secret for display as `head…tail` (e.g. `sk-or-…4f2a`) — enough of
/// the prefix to recognize which key it is, never enough to reconstruct it.
/// Values too short to mask usefully are replaced entirely.
pub(super) fn mask_secret(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 9 {
        return "…".to_string();
    }
    let head: String = chars[..5].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}
