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
/// Only meaningful for pubkeys; session identity is shown through the agent's
/// instance label (`haiku`, `haiku1`, ...), not a shortened or generated id.
pub fn pubkey_short(id: &str) -> String {
    id.chars().take(8).collect()
}

/// A short prefix of a message/event id (its first 6 hex chars) — cheap to
/// include inline in agent-facing context. The daemon resolves any
/// unambiguous prefix back to the full event on `channel read --id`, so this
/// stays round-trippable without spending tokens on a full 64-char id.
pub fn short_id(id: &str) -> String {
    id.chars().take(6).collect()
}

const CODE_WORDS_A: [&str; 32] = [
    "amber", "atlas", "birch", "cedar", "clay", "comet", "coral", "dawn", "ember", "flint", "gale",
    "hazel", "indigo", "ivy", "jet", "juno", "kite", "lark", "lumen", "maple", "nova", "onyx",
    "opal", "pearl", "quill", "quinn", "reed", "sable", "slate", "tide", "willow", "zephyr",
];

const CODE_WORDS_B: [&str; 32] = [
    "arrow", "bravo", "canyon", "cliff", "delta", "drift", "echo", "falcon", "fjord", "grove",
    "harbor", "haven", "kilo", "lima", "meadow", "mesa", "orbit", "pace", "peak", "quest", "range",
    "ridge", "river", "shore", "sierra", "spark", "summit", "tango", "trail", "vale", "wave",
    "yonder",
];

/// Deterministic, memorable short code for a long/opaque id (e.g. a raw
/// session id) — `word-word-NNN`. Cheaper to read, say, and hold in context
/// than a hex/UUID correlation handle. `DefaultHasher` is fixed-seed (see
/// [`session_room_id`]), so the same input always maps to the same code, both
/// within a run and across daemon restarts. Purely a display transform: the
/// underlying id is unchanged and this code is never resolved back to it.
pub fn friendly_short_code(id: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    let h = hasher.finish();
    let a = CODE_WORDS_A[(h as usize) % CODE_WORDS_A.len()];
    let b = CODE_WORDS_B[((h >> 20) as usize) % CODE_WORDS_B.len()];
    let n = (h >> 40) % 1000;
    format!("{a}-{b}-{n:03}")
}

pub const CHAT_RENDER_WORD_LIMIT: usize = 300;

/// `channel send` refuses to publish a message longer than this many characters
/// unless the caller passes `--long-message`.
pub const CHAT_WRITE_CHAR_LIMIT: usize = 600;

pub fn truncate_words(text: &str, limit: usize) -> (String, bool) {
    let mut words = text.split_whitespace();
    let kept: Vec<&str> = words.by_ref().take(limit).collect();
    if words.next().is_none() {
        return (text.trim().to_string(), false);
    }
    (format!("{}...", kept.join(" ")), true)
}

/// True when `text`, trimmed, starts with `<` — the shape of harness-injected
/// control content (task-completion notifications, system reminders,
/// command-output wrappers, ...) as opposed to text a human actually typed.
/// Human prose never starts with `<`; harness envelopes always do. Such a
/// prompt is harness plumbing, not human speech, and must not be mirrored
/// into chat as if it were (issue: raw `<task-notification>` blobs were
/// getting posted into the channel verbatim).
///
/// Deliberately just a leading-`<` check, not "one well-formed wrapped
/// element": some harness envelopes are several sibling top-level tags
/// (Claude Code's slash-command expansion emits
/// `<command-message>...</command-message><command-name>...</command-name>`,
/// two elements, not one), so requiring a single matching open/close tag
/// misses those. Mirrors `proactive-context`'s `visible_text`, validated
/// against real sessions there. The accepted false positive is a human
/// prompt that happens to start with a literal `<` — rare enough that
/// harness content never leaking into chat matters more.
pub fn is_harness_envelope(text: &str) -> bool {
    text.trim_start().starts_with('<')
}

/// A session identifier. Wraps the raw id stored in SQLite. `Display` preserves
/// the raw id for correlation only; user-facing identity belongs to the agent
/// instance label, not a generated session alias.
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
        f.write_str(&self.0)
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

/// Convert a human-readable host label (e.g. "pablos' laptop") into a URL-safe
/// slug (e.g. "pablos-laptop"). This is only for legacy/local normalization
/// needs; public agent/backend labels preserve config.json `backendName`.
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

/// True when `s` has the exact shape minted by [`opaque_group_id`]: 8
/// lowercase-hex chars (`[0-9a-f]{8}`). Used at the ONE shared channel resolver
/// to discriminate an already-RESOLVED opaque channel id from a human channel
/// NAME: an 8-hex value that misses the local cache is a not-yet-materialized id
/// (a relay race), never a name to mint a literal-named channel for. A human
/// handle that happens to be exactly 8 lowercase-hex chars is astronomically
/// unlikely, so the heuristic is safe.
pub fn is_opaque_group_id(s: &str) -> bool {
    s.len() == 8
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Deterministic id for a per-session room (issue #6): `session-` followed by
/// six `[a-z0-9]` chars (base36) derived from a stable hash of the session's
/// `anchor` (resume token / harness id / pid).
///
/// The id does NOT prefix the work-root channel name: a session channel is
/// already nested under its channel via the NIP-29 `parent` tag, so repeating
/// the channel in the id is redundant. The child→parent link is the relay's
/// kind:39000 `parent` tag, materialized into `relay_channels.parent` — never
/// inferred from the id's shape.
///
/// The short hash keeps the room id readable while the `session-` prefix makes
/// the scope explicit in prompts, status lines, and injected mentions.
/// Deterministic (`DefaultHasher::new()` uses fixed keys) so a resumed session
/// re-derives the same room; `anchor` is hashed, never embedded verbatim in
/// the channel id.
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
    fn short_id_truncates_to_six() {
        assert_eq!(short_id("0123456789abcdef"), "012345");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn friendly_short_code_is_deterministic_and_shaped() {
        let a = friendly_short_code("te-18be8e4753ae57e0-dc");
        let b = friendly_short_code("te-18be8e4753ae57e0-dc");
        assert_eq!(a, b, "same input must always map to the same code");
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.len(), 3, "expected word-word-NNN, got {a:?}");
        assert_eq!(parts[2].len(), 3);
        assert!(parts[2].chars().all(|c| c.is_ascii_digit()));
        assert_ne!(
            friendly_short_code("session-a"),
            friendly_short_code("session-b"),
            "distinct ids should (almost always) map to distinct codes"
        );
    }

    #[test]
    fn is_harness_envelope_detects_leading_angle_bracket() {
        assert!(is_harness_envelope(
            "<task-notification>\n<task-id>abc</task-id>\n</task-notification>"
        ));
        assert!(is_harness_envelope(
            "<system-reminder>careful</system-reminder>"
        ));
        // Whitespace around the whole message is ignored.
        assert!(is_harness_envelope(
            "  \n<system-reminder>careful</system-reminder>\n  "
        ));
        // Sibling top-level tags (Claude Code slash-command expansion), not one
        // wrapped element — still harness content, must still be caught.
        assert!(is_harness_envelope(
            "<command-message>running</command-message><command-name>foo</command-name>"
        ));
        // Opens a tag but never closes it — still harness content.
        assert!(is_harness_envelope("<task-notification>partial"));
    }

    #[test]
    fn is_harness_envelope_rejects_genuine_human_text() {
        // A mid-sentence `<` doesn't trigger — only a *leading* one does.
        assert!(!is_harness_envelope("fix the bug in <Foo/> please"));
        assert!(!is_harness_envelope("plain text prompt"));
        assert!(!is_harness_envelope(""));
    }

    #[test]
    fn session_id_display_preserves_raw_id() {
        let sid = SessionId::from("local-session");
        assert_eq!(sid.to_string(), "local-session");
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
    fn opaque_group_id_is_8_hex() {
        let id = opaque_group_id();
        assert_eq!(id.len(), 8, "got {id}");
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "non-hex id {id}");
    }

    #[test]
    fn is_opaque_group_id_discriminates_ids_from_names() {
        assert!(is_opaque_group_id("2f1cd36f"));
        assert!(is_opaque_group_id(&opaque_group_id()));
        // Genuine human handles are not the opaque shape.
        assert!(!is_opaque_group_id("backlog-work"));
        assert!(!is_opaque_group_id("planning")); // 8 chars but not all hex
        assert!(!is_opaque_group_id("2F1CD36F")); // uppercase hex is not our shape
        assert!(!is_opaque_group_id("2f1cd3")); // too short
        assert!(!is_opaque_group_id("2f1cd36ff")); // too long
        assert!(!is_opaque_group_id(""));
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
        // No channel name anywhere in the id — the room is nested via parent.
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
