//! Tiny shared helpers.

use std::time::{SystemTime, UNIX_EPOCH};

/// Current unix time in seconds.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
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

/// Derive a fresh child (sub-)group id of the shape `<slug>-<random8>`, where
/// `<slug>` is [`slugify_host`] applied to the human display `name` and
/// `<random8>` is 8 lowercase hex chars (4 random bytes). The random suffix keeps
/// subgroup ids unique even when several share a name, so the relay's
/// client-chosen group id never collides across creates.
///
/// Randomness is sourced from a freshly generated keypair's public key
/// (a Schnorr x-coordinate, effectively uniform); we take its first 4 bytes.
/// This avoids a direct `rand` dependency while staying cryptographically random.
pub fn child_group_id(name: &str) -> String {
    let slug = slugify_host(name);
    let pk = nostr_sdk::prelude::Keys::generate().public_key().to_hex();
    // First 8 hex chars == first 4 bytes; already lowercase from `to_hex`.
    let rand8: String = pk.chars().take(8).collect();
    format!("{slug}-{rand8}")
}

/// Derive a *deterministic* per-session room id of the same `<slug>-<hex8>`
/// shape as [`child_group_id`], but with the hex suffix derived from a stable
/// `anchor` (the session's resume token / harness id) instead of randomness.
///
/// Determinism is the point: a resumed session re-enters this path with the
/// same `anchor` and so rejoins the SAME room (issue #6: "resume reuses the
/// room"), rather than minting a fresh subgroup each time. `DefaultHasher::new()`
/// uses fixed keys, so the suffix is stable across daemon restarts.
///
/// `anchor` is hashed, never embedded verbatim, so a local-only handle never
/// reaches the wire (issue #5: "no session_id on the wire").
pub fn child_group_id_from_anchor(name: &str, anchor: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let slug = slugify_host(name);
    let mut hasher = DefaultHasher::new();
    anchor.hash(&mut hasher);
    let hex8 = format!("{:08x}", hasher.finish() as u32);
    format!("{slug}-{hex8}")
}

/// True when `project` is the work-root `name` itself, or a per-session room
/// minted under it — i.e. `<slugify(name)>-<hex8>` (the shape produced by
/// [`child_group_id`] / [`child_group_id_from_anchor`]).
///
/// Host-side session resolution needs this because a human-initiated session
/// (someone ran `claude`/`codex` directly, no `TENEX_EDGE_CHANNEL`) is stored
/// under its minted room, but the same terminal's later `tenex-edge` verbs only
/// know the bare work-root from `cwd`. Matching the room lets those verbs find
/// the session they belong to. The suffix is required to be exactly 8 lowercase
/// hex chars so a sibling project that merely shares the prefix can't match.
pub fn is_session_room_of(project: &str, name: &str) -> bool {
    let slug = slugify_host(name);
    if project == name || project == slug {
        return true;
    }
    project
        .strip_prefix(&slug)
        .and_then(|rest| rest.strip_prefix('-'))
        .is_some_and(|hex| {
            hex.len() == 8 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
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
    fn child_group_id_shape() {
        let id = child_group_id("Subgroup Support");
        assert!(id.starts_with("subgroup-support-"), "got {id}");
        let suffix = id.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 8, "got {id}");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex suffix in {id}"
        );
    }

    #[test]
    fn child_group_id_unique() {
        let a = child_group_id("Subgroup Support");
        let b = child_group_id("Subgroup Support");
        assert_ne!(a, b);
    }

    #[test]
    fn child_group_id_from_anchor_shape() {
        let id = child_group_id_from_anchor("My Repo", "sess-abc-123");
        assert!(id.starts_with("my-repo-"), "got {id}");
        let suffix = id.rsplit('-').next().unwrap();
        assert_eq!(suffix.len(), 8, "got {id}");
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex suffix in {id}"
        );
    }

    #[test]
    fn child_group_id_from_anchor_is_deterministic() {
        // Same anchor → same id (so a resumed session rejoins the SAME room).
        let a = child_group_id_from_anchor("my-repo", "sess-abc-123");
        let b = child_group_id_from_anchor("my-repo", "sess-abc-123");
        assert_eq!(a, b);
    }

    #[test]
    fn child_group_id_from_anchor_varies_by_anchor() {
        // Different sessions in the same repo get different rooms.
        let a = child_group_id_from_anchor("my-repo", "sess-aaa");
        let b = child_group_id_from_anchor("my-repo", "sess-bbb");
        assert_ne!(a, b);
    }

    #[test]
    fn child_group_id_from_anchor_does_not_embed_anchor() {
        // The anchor (a local-only handle) is hashed, never carried verbatim.
        let id = child_group_id_from_anchor("my-repo", "secret-resume-token-xyz");
        assert!(!id.contains("secret-resume-token-xyz"), "got {id}");
    }

    #[test]
    fn is_session_room_of_matches_bare_and_minted_rooms() {
        // The bare work-root itself, and any room minted beneath it, match.
        assert!(is_session_room_of("lsjkd", "lsjkd"));
        assert!(is_session_room_of("lsjkd-2d3145b3", "lsjkd"));
        assert!(is_session_room_of(
            &child_group_id_from_anchor("lsjkd", "sess-1"),
            "lsjkd"
        ));
        // A repo whose name already contains a hyphen still works.
        assert!(is_session_room_of("tenex-edge-86b961bb", "tenex-edge"));
    }

    #[test]
    fn is_session_room_of_rejects_unrelated_and_prefix_lookalikes() {
        // Different project entirely.
        assert!(!is_session_room_of("other-2d3145b3", "lsjkd"));
        // A sibling project that merely shares the prefix but whose suffix is
        // not exactly 8 hex chars must NOT match.
        assert!(!is_session_room_of("lsjkd-frontend", "lsjkd"));
        assert!(!is_session_room_of("lsjkd-2d3145b", "lsjkd")); // 7 chars
        assert!(!is_session_room_of("lsjkd-2d3145b3a", "lsjkd")); // 9 chars
        assert!(!is_session_room_of("lsjkd2d3145b3", "lsjkd")); // missing hyphen
    }

    #[test]
    fn now_is_after_2020() {
        assert!(now_secs() > 1_577_836_800);
    }
}
