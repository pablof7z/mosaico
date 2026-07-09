//! The SINGLE canonical way to refer to agents and sessions across every
//! tenex-edge interface (identifier standardization).
//!
//! Identity is the durable AGENT-INSTANCE = `(slug, backend-label)` →
//! **`agent@backend-label`** (e.g. `codex@laptop`, `haiku1@myBackend`). A single
//! run (SESSION) is only a correlation handle (the raw `session_id`); it is never
//! a separate display name and is never accepted as a chat target.
//!
//! Rules that hold EVERYWHERE:
//!   - `@` always means backend label, never channel. An agent is
//!     `(slug, backend-label)`; channel is only where a message goes, never who it
//!     is.
//!   - backend labels are config.json `backendName` values and are preserved
//!     exactly after trimming. They are not DNS hostnames and are not slugified.
//!   - identity is the agent-instance label, resolving to the instance's selected
//!     pubkey; correlation is the raw `session_id`.
//!
//! Every renderer formats via [`agent_label`] / [`session_label`]; every input
//! is classified via [`parse_ref`]. Nothing hand-rolls `format!("{slug}@…")`.

/// Canonical label for a durable agent: `agent@backend-label`. When the backend
/// label is unknown (empty), degrades to the bare `agent` rather than `agent@`.
pub fn agent_label(slug: &str, host: &str) -> String {
    let host = host.trim();
    if host.is_empty() {
        slug.to_string()
    } else {
        format!("{slug}@{host}")
    }
}

/// Strip this backend suffix from a kind:0 profile name to recover the routing
/// slug. Legacy profiles that publish a bare slug pass through unchanged.
pub fn slug_from_profile_name(name: &str, host: &str) -> String {
    let name = name.trim();
    let host = host.trim();
    if host.is_empty() {
        return name.to_string();
    }
    let suffix = format!("@{host}");
    match name.strip_suffix(&suffix) {
        Some(slug) if !slug.trim().is_empty() => slug.trim().to_string(),
        _ => name.to_string(),
    }
}

/// Backend-aware agent reference as seen from `local_host`: bare `slug` when the
/// agent is on the local backend (or its backend label is unknown), else
/// `slug@backend-label`.
///
/// This is the token an operator/agent TYPES to address the peer: `developer`
/// names the local developer, while `developer@myBackend` singles out a same-slug
/// agent on another backend. Display layers add the `@` mention sigil on top
/// (`@developer` / `@developer@tower`).
pub fn agent_ref_from(slug: &str, host: &str, local_host: &str) -> String {
    let host = host.trim();
    if host.is_empty() || host == local_host.trim() {
        slug.to_string()
    } else {
        agent_label(slug, host)
    }
}

/// Display for a sender on an envelope "From" line: the agent-instance label
/// `agent@backend-label`. When the agent slug is unknown, degrades to the raw
/// `session_id` as a bare correlation handle.
pub fn session_label(session_id: &str, slug: &str, host: &str) -> String {
    if slug.is_empty() {
        session_id.to_string()
    } else {
        agent_label(slug, host)
    }
}

/// A short prefix of a message/event id for envelope IDs and send confirmations.
pub fn event_short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// A syntactically-classified identifier token (the input side). Resolution
/// against the store happens in the daemon; this is the pure classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    /// `agent@backend-label` — a durable agent on a specific backend.
    Agent { slug: String, host: String },
    /// A 64-char hex pubkey or `npub1…`.
    Pubkey(String),
    /// Anything else: a session (canonical id / harness alias / id prefix) OR a
    /// bare agent-instance label. The daemon resolver disambiguates by trying
    /// session lookups first, then `slug@<local-host>`.
    Token(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBackendRef {
    pub slug: String,
    /// Backend label from config.json `backendName`, never a DNS/OS hostname.
    pub backend: Option<String>,
}

/// Parse `agent[@backend-label]` for invite/orchestration surfaces. Unlike
/// display-oriented `agent@backend-label` parsing, the right side is a backend config
/// label and is preserved exactly after trimming.
pub fn parse_agent_backend_ref(spec: &str) -> Option<AgentBackendRef> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    match spec.rsplit_once('@') {
        Some((slug, backend)) if !slug.trim().is_empty() && !backend.trim().is_empty() => {
            Some(AgentBackendRef {
                slug: slug.trim().to_string(),
                backend: Some(backend.trim().to_string()),
            })
        }
        Some(_) => None,
        None => Some(AgentBackendRef {
            slug: spec.to_string(),
            backend: None,
        }),
    }
}

/// Parse an identifier token into a syntactic [`Ref`]. `@` ALWAYS means backend
/// label.
pub fn parse_ref(token: &str) -> Ref {
    let t = token.trim();
    if let Some((slug, host)) = t.rsplit_once('@') {
        let slug = slug.trim();
        let host = host.trim();
        if !slug.is_empty() && !host.is_empty() {
            return Ref::Agent {
                slug: slug.to_string(),
                host: host.to_string(),
            };
        }
    }
    if is_pubkey(t) {
        return Ref::Pubkey(t.to_string());
    }
    Ref::Token(t.to_string())
}

fn is_pubkey(s: &str) -> bool {
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())) || s.starts_with("npub1")
}

/// Extract inline `@<agent-instance-label>` mentions from free chat text, in
/// order of appearance, deduped. A mention is `@` followed by an agent-instance
/// label token — a run of `[A-Za-z0-9._-]`, optionally backend-qualified as
/// `label@backend-label` (so both `@haiku1` and `@haiku@laptop` are captured; the
/// resolver understands `agent@backend-label`). Used so
/// `channel send "hey @haiku1"` routes to that instance. Trailing punctuation (`,`,
/// `.` at the end of a word, `!`, `?`, `:`) is ignored. Tokens that don't resolve
/// are silently treated as no mention.
pub fn extract_mentions(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in body.split(|c: char| c.is_whitespace()) {
        let Some(at) = raw.find('@') else { continue };
        if at > 0
            && raw[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        // Take the run of label/host characters immediately after the '@' sigil,
        // allowing a single internal '@' for a host-qualified `label@host`.
        let after = &raw[at + 1..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@')))
            .unwrap_or(after.len());
        // Drop trailing dots/sigils so `@haiku1.` and `@haiku@` degrade cleanly.
        let candidate = after[..end].trim_end_matches(['.', '@']);
        if !candidate.is_empty() && !out.iter().any(|m| m == candidate) {
            out.push(candidate.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_label_preserves_backend_label() {
        assert_eq!(agent_label("codex", "myBackend"), "codex@myBackend");
        assert_eq!(agent_label("claude", "laptop"), "claude@laptop");
    }

    #[test]
    fn slug_from_profile_name_strips_matching_backend_suffix() {
        assert_eq!(
            slug_from_profile_name("developer1@remoteBackend", "remoteBackend"),
            "developer1"
        );
        assert_eq!(
            slug_from_profile_name("developer1", "remoteBackend"),
            "developer1"
        );
        assert_eq!(
            slug_from_profile_name("developer1@otherBackend", "remoteBackend"),
            "developer1@otherBackend"
        );
        assert_eq!(
            slug_from_profile_name("developer1@remoteBackend", ""),
            "developer1@remoteBackend"
        );
    }

    #[test]
    fn agent_ref_from_is_bare_local_and_qualified_remote() {
        // Same backend label → bare slug; you'd type just `developer`.
        assert_eq!(agent_ref_from("developer", "laptop", "laptop"), "developer");
        // Unknown backend → bare (can't qualify what we don't know).
        assert_eq!(agent_ref_from("developer", "", "laptop"), "developer");
        // Different backend → backend-qualified so a same-slug remote stays distinct.
        assert_eq!(
            agent_ref_from("developer", "myBackend", "laptop"),
            "developer@myBackend"
        );
    }

    #[test]
    fn session_label_is_agent_at_backend_label() {
        // The sender's "From" identity is the agent-instance label.
        assert_eq!(session_label("te-abc-0", "codex", "laptop"), "codex@laptop");
        // Unknown slug degrades to the raw session id as a correlation handle.
        assert_eq!(session_label("te-abc-0", "", "laptop"), "te-abc-0");
    }

    #[test]
    fn event_short_id_truncates_to_eight() {
        assert_eq!(event_short_id("0123456789abcdef"), "01234567");
        assert_eq!(event_short_id("abc"), "abc");
    }

    #[test]
    fn parse_at_is_backend_label_not_channel() {
        match parse_ref("codex@myBackend") {
            Ref::Agent { slug, host } => {
                assert_eq!(slug, "codex");
                assert_eq!(host, "myBackend");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn agent_backend_ref_preserves_backend_label() {
        let r = parse_agent_backend_ref("claude@myBackend").unwrap();
        assert_eq!(r.slug, "claude");
        assert_eq!(r.backend.as_deref(), Some("myBackend"));

        let local = parse_agent_backend_ref("codex").unwrap();
        assert_eq!(local.slug, "codex");
        assert_eq!(local.backend, None);

        assert!(parse_agent_backend_ref("claude@").is_none());
        assert!(parse_agent_backend_ref("@laptop").is_none());
    }

    #[test]
    fn parse_pubkey_and_token() {
        let hex = "a".repeat(64);
        assert!(matches!(parse_ref(&hex), Ref::Pubkey(_)));
        assert!(matches!(parse_ref("npub1abcdef"), Ref::Pubkey(_)));
        assert!(matches!(parse_ref("haiku1"), Ref::Token(_)));
        assert!(matches!(parse_ref("codex"), Ref::Token(_)));
    }

    #[test]
    fn extract_inline_mentions() {
        // Agent-instance labels (bare and ordinal) are now accepted mentions.
        assert_eq!(
            extract_mentions("hey @haiku1 and @codex, look"),
            vec!["haiku1".to_string(), "codex".to_string()]
        );
        // Host-qualified `label@host` is captured intact for the resolver.
        assert_eq!(
            extract_mentions("ping @claude@tower please"),
            vec!["claude@tower".to_string()]
        );
        // Trailing punctuation is trimmed.
        assert_eq!(extract_mentions("ping @codex."), vec!["codex".to_string()]);
        // Email-ish substrings are not mentions.
        assert!(extract_mentions("email dev@example.com please").is_empty());
        // dedup
        assert_eq!(
            extract_mentions("@haiku1 @haiku1"),
            vec!["haiku1".to_string()]
        );
    }
}
