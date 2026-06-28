//! Tiny shared helpers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current unix time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current unix time in milliseconds.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format a unix timestamp as local-time `YYYY-MM-DD HH:MM` (via `localtime_r`,
/// so it honors the daemon machine's timezone — the wall-clock an agent expects).
pub fn format_local_datetime(unix_secs: u64) -> String {
    // SAFETY: `localtime_r` writes into a zeroed `tm` we own; no shared state.
    unsafe {
        let t = unix_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "unknown".to_string();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
        )
    }
}

/// Format a unix timestamp (milliseconds) as local-time `YYYY-MM-DD HH:MM:SS.mmm`.
pub fn format_local_datetime_ms(unix_millis: u64) -> String {
    let secs = unix_millis / 1000;
    let ms = unix_millis % 1000;
    // SAFETY: `localtime_r` writes into a zeroed `tm` we own; no shared state.
    unsafe {
        let t = secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return "unknown".to_string();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            ms,
        )
    }
}

/// Human-friendly relative time: `just now` (<60s), `N min ago`, `N hour(s) ago`,
/// `yesterday`, then `N days ago`.
pub fn relative_time(then: u64, now: u64) -> String {
    let d = now.saturating_sub(then);
    if d < 60 {
        "just now".to_string()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 86_400 {
        let h = d / 3600;
        format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
    } else if d < 172_800 {
        "yesterday".to_string()
    } else {
        format!("{} days ago", d / 86_400)
    }
}

/// The ` [N file(s) dirty]` suffix for an envelope's Branch line. Empty when the
/// sender's working tree was clean (`n == 0`), so a clean branch renders bare.
pub fn dirty_label(n: u32) -> String {
    match n {
        0 => String::new(),
        1 => " [1 file dirty]".to_string(),
        _ => format!(" [{n} files dirty]"),
    }
}

/// A short, human-targetable prefix of a PUBKEY (its first 8 hex chars).
/// Only meaningful for pubkeys — never use it to display a session id (use the
/// `SessionId` newtype, whose `Display` routes through `session_codename`).
pub fn pubkey_short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// The NATO phonetic alphabet — the word stems of a session codename.
const CODENAME_WORDS: [&str; 26] = [
    "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra", "tango",
    "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
];

/// Derive a stable, human-friendly **codename** for a session ID: a NATO
/// phonetic word plus a four-digit number, e.g. `bravo4217` or `echo0163`.
/// Replaces the old 6-char hex hash — a codename is just as stable (same id →
/// same codename) but easy to say aloud and remember.
///
/// The space is 26×10000 = 260000 codenames. That is plenty for the sessions a
/// fabric ever holds, but it is NOT collision-free at scale; it is a
/// display/addressing convenience, never an identity (the canonical session id
/// remains the source of truth).
pub fn session_codename(session_id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    let hash = hasher.finish();

    let word = CODENAME_WORDS[(hash % CODENAME_WORDS.len() as u64) as usize];
    let num = (hash / CODENAME_WORDS.len() as u64) % 10_000;
    format!("{word}{num:04}")
}

/// Heuristic: does `s` look like a session codename (`<nato-word><digits>`,
/// e.g. `bravo4217`)? Used to disambiguate a bare token between a session
/// codename and a durable agent slug when resolving an identifier. A leading
/// NATO word immediately followed by one-or-more digits and nothing else.
pub fn looks_like_codename(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let split = lower.find(|c: char| c.is_ascii_digit());
    let Some(idx) = split else { return false };
    let (word, digits) = lower.split_at(idx);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && CODENAME_WORDS.contains(&word)
}

/// Derive a short title from a raw user prompt: take the first non-empty line,
/// strip leading markdown sigils (#, *, -, >), and cap at 60 chars on a word
/// boundary. Returns an empty string when nothing meaningful remains.
pub fn titleize_prompt(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches(['#', '*', '-', '>', ' ', '\t'])
        .trim();
    if line.is_empty() {
        return String::new();
    }
    if line.len() <= 60 {
        return line.to_string();
    }
    match line[..60].rfind(' ') {
        Some(i) => line[..i].to_string(),
        None => line[..60].to_string(),
    }
}

/// A session identifier. Wraps the raw id (a UUID-shaped string stored verbatim
/// in SQLite and carried on the wire) but its `Display` deliberately renders the
/// stable `session_codename` (e.g. `bravo4217`), NOT the raw id. This makes it
/// structurally impossible to print a session id through `pubkey_short` (the
/// wrong formatter): any `{session_id}` in a format string yields the codename.
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
        f.write_str(&session_codename(&self.0))
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

/// A fresh OPAQUE channel/group id: 8 lowercase hex chars (4 random bytes) from a
/// freshly generated keypair's public key. NEVER derived from the channel's name —
/// the human handle lives in the kind:39000 `name` tag, while this id is the
/// durable, collision-resistant key the relay addresses (the NIP-29 `h`/`d`).
///
/// Randomness is sourced from a Schnorr x-coordinate (effectively uniform); we
/// take its first 4 bytes. This avoids a direct `rand` dependency while staying
/// cryptographically random.
pub fn opaque_group_id() -> String {
    let pk = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
    // First 8 hex chars == first 4 bytes; already lowercase from `to_hex`.
    pk.chars().take(8).collect()
}

/// Deterministic id for a per-session room (issue #6): `session-` followed by
/// six `[a-z0-9]` chars (base36) derived from a stable hash of the session's
/// `anchor` (resume token / harness id / pid).
///
/// The id does NOT prefix the work-root project name: a session channel is
/// already nested under its project via the NIP-29 `parent` tag, so repeating
/// the project in the id is redundant. The child→parent link is the relay's
/// kind:39000 `parent` tag, materialized into `relay_channels.parent` — never
/// inferred from the id's shape.
///
/// The short hash keeps the room id readable while the `session-` prefix makes
/// the scope explicit in prompts, status lines, and injected mentions.
/// Deterministic (`DefaultHasher::new()` uses fixed keys) so a resumed session
/// re-derives the same room; `anchor` is hashed, never embedded verbatim (no
/// session_id on the wire, issue #5).
pub fn session_room_id(anchor: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut hasher = DefaultHasher::new();
    anchor.hash(&mut hasher);
    let mut n = hasher.finish();
    let mut id = [0u8; 6];
    for slot in id.iter_mut() {
        *slot = ALPHABET[(n % 36) as usize];
        n /= 36;
    }
    // Safe: every byte is an ASCII char from ALPHABET.
    format!("session-{}", String::from_utf8(id.to_vec()).unwrap())
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
    fn session_id_display_uses_codename() {
        let sid = SessionId::from("local-session");
        assert_eq!(sid.to_string(), session_codename("local-session"));
        assert_eq!(sid.as_str(), "local-session");
    }

    #[test]
    fn session_codename_is_word_plus_four_digits() {
        let code = session_codename("some-session-uuid");
        // Stable across calls.
        assert_eq!(code, session_codename("some-session-uuid"));
        // Shape: a phonetic word stem followed by exactly four digits.
        let digits: String = code.chars().rev().take(4).collect();
        assert!(digits.chars().all(|c| c.is_ascii_digit()), "got {code}");
        let word: String = code[..code.len() - 4].to_string();
        assert!(CODENAME_WORDS.contains(&word.as_str()), "got {code}");
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
    fn opaque_group_id_is_8_hex() {
        let id = opaque_group_id();
        assert_eq!(id.len(), 8, "got {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex id {id}"
        );
    }

    #[test]
    fn opaque_group_id_unique_and_nameless() {
        // Minted fresh each call (never derived from any name).
        let a = opaque_group_id();
        let b = opaque_group_id();
        assert_ne!(a, b);
    }

    #[test]
    fn session_room_id_shape() {
        let id = session_room_id("sess-abc-123");
        assert!(id.starts_with("session-"), "got {id}");
        let suffix = id.trim_start_matches("session-");
        assert_eq!(suffix.len(), 6, "got {id}");
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "non-[a-z0-9] char in {id}"
        );
        // No project name anywhere in the id — the room is nested via parent.
        assert!(!session_room_id("my-repo-anchor").contains("my-repo"));
    }

    #[test]
    fn session_room_id_is_deterministic() {
        // Same anchor → same id (so a resumed session rejoins the SAME room).
        assert_eq!(
            session_room_id("sess-abc-123"),
            session_room_id("sess-abc-123")
        );
    }

    #[test]
    fn session_room_id_varies_by_anchor() {
        assert_ne!(session_room_id("sess-aaa"), session_room_id("sess-bbb"));
    }

    #[test]
    fn session_room_id_does_not_embed_anchor() {
        // The anchor (a local-only handle) is hashed, never carried verbatim.
        let id = session_room_id("secret-resume-token-xyz");
        assert!(!id.contains("secret-resume-token-xyz"), "got {id}");
    }

    #[test]
    fn now_is_after_2020() {
        assert!(now_secs() > 1_577_836_800);
    }
}
