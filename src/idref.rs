//! The canonical reference helpers for tenex-edge identities.
//!
//! User-facing session identity is **`agent/session`** (for example
//! `codex/echo123`). Backend-qualified agent references (`agent@backend-label`)
//! remain an operator/backend selection syntax for invite and legacy lookup
//! paths, not the public session handle.
//!
//! Rules that hold everywhere:
//!   - Agent session handles are rendered with [`session_handle`].
//!   - Backend labels are config.json `backendName` values and are preserved
//!     exactly after trimming. They are not DNS hostnames and are not slugified.
//!   - Raw session ids are internal correlation handles. A friendly session
//!     codename may appear as the right side of `agent/session`.
//!
//! Every renderer formats through this module; every input is classified via
//! [`parse_ref`] or normalized through the session-handle helpers.

/// Canonical user-facing session handle: `agentSlug/session`.
pub fn session_handle(agent_slug: &str, session: &str) -> String {
    let agent_slug = agent_slug.trim();
    let session = session.trim();
    if session.is_empty() {
        return agent_slug.to_string();
    }
    if agent_slug.is_empty() {
        return session.to_string();
    }
    let prefix = format!("{agent_slug}/");
    if session.starts_with(&prefix) {
        session.to_string()
    } else {
        format!("{agent_slug}/{session}")
    }
}

/// Parse `agentSlug/session` into its two routing-visible parts.
pub fn parse_session_handle(handle: &str) -> Option<(&str, &str)> {
    let handle = handle.trim();
    let (agent_slug, session) = handle.split_once('/')?;
    let agent_slug = agent_slug.trim();
    let session = session.trim();
    if agent_slug.is_empty() || session.is_empty() || session.contains('/') {
        return None;
    }
    Some((agent_slug, session))
}

/// Convert a kind:0 `name` plus tags into the canonical session handle.
///
/// New profiles already publish `agent/session`. Legacy profiles published
/// `session@backend`; when an `agent-slug` tag is present, normalize that cache
/// row to `agent/session` while still accepting the old event.
pub fn session_handle_from_profile_name(name: &str, host: &str, agent_slug: &str) -> String {
    let name = name.trim();
    if parse_session_handle(name).is_some() {
        return name.to_string();
    }
    let session = slug_from_profile_name(name, host);
    if parse_session_handle(&session).is_some() {
        return session;
    }
    if agent_slug.trim().is_empty() {
        session
    } else {
        session_handle(agent_slug, &session)
    }
}

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

/// Display for a sender on an envelope "From" line. `slug` is expected to be an
/// already-normalized session handle; legacy callers that still provide a bare
/// slug plus host degrade through the backend-qualified form.
pub fn session_label(session_id: &str, slug: &str, host: &str) -> String {
    if slug.is_empty() {
        session_id.to_string()
    } else if parse_session_handle(slug).is_some() {
        slug.to_string()
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

/// Extract inline `@<agent-session-handle>` mentions from free chat text, in
/// order of appearance, deduped. A mention is `@` followed by a run of
/// `[A-Za-z0-9._-/]`, with a single legacy `@` still accepted for
/// `agent@backend-label`. Trailing punctuation is ignored. Tokens that don't
/// resolve are silently treated as no mention.
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
        // Take the run of handle characters immediately after the '@' sigil,
        // allowing a single internal '@' for legacy host-qualified labels.
        let after = &raw[at + 1..];
        let end = after
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '@'))
            })
            .unwrap_or(after.len());
        // Drop trailing separators so `@codex/echo.` and `@haiku@` degrade cleanly.
        let candidate = after[..end].trim_end_matches(['.', '@', '/']);
        if !candidate.is_empty() && !out.iter().any(|m| m == candidate) {
            out.push(candidate.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests;
