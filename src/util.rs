//! Tiny shared helpers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current unix time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A short, human-targetable prefix of a PUBKEY (its first 8 hex chars).
/// Only meaningful for pubkeys — never use it to display a session id (use the
/// `SessionId` newtype, whose `Display` routes through `session_short_code`).
pub fn pubkey_short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Hash a session ID to a unique, stable 6-character code.
/// Deterministic hash ensures the same session_id always gets the same code.
pub fn session_short_code(session_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    let hash = hasher.finish();

    // Format as 6-char hex for visual distinction and stable output
    format!("{:06x}", hash % 0x1_000_000)
}

/// A session identifier. Wraps the raw id (a UUID-shaped string stored verbatim
/// in SQLite and carried on the wire) but its `Display` deliberately renders the
/// stable 6-char `session_short_code`, NOT the raw id. This makes it structurally
/// impossible to print a session id through `pubkey_short` (the wrong formatter):
/// any `{session_id}` in a format string yields the short code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn into_string(self) -> String {
        self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&session_short_code(&self.0))
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Convert a human-readable host label (e.g. "pablos' laptop") into a
/// URL-safe slug (e.g. "pablos-laptop") suitable for use in `agent@host`
/// addressing.
pub fn slugify_host(host: &str) -> String {
    let slug: String = host
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive hyphens and strip leading/trailing ones.
    let mut out = String::with_capacity(slug.len());
    let mut prev_hyphen = true; // treat start as hyphen to strip leading ones
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                out.push('-');
                prev_hyphen = true;
            }
        } else {
            out.push(c);
            prev_hyphen = false;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubkey_short_truncates() {
        assert_eq!(pubkey_short("0123456789abcdef"), "01234567");
        assert_eq!(pubkey_short("abc"), "abc");
    }

    #[test]
    fn session_id_display_uses_short_code() {
        let sid = SessionId::from("local-session");
        assert_eq!(sid.to_string(), session_short_code("local-session"));
        assert_eq!(sid.as_str(), "local-session");
    }

    #[test]
    fn slugify_host_normalizes() {
        assert_eq!(slugify_host("pablos' laptop"), "pablos-laptop");
        assert_eq!(slugify_host("My MacBook Pro!"), "my-macbook-pro");
        assert_eq!(slugify_host("tower"), "tower");
        assert_eq!(slugify_host("  "), "unknown");
        assert_eq!(slugify_host("abc--def"), "abc-def");
    }

    #[test]
    fn now_is_after_2020() {
        assert!(now_secs() > 1_577_836_800);
    }
}
