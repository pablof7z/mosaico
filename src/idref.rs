//! The canonical reference helpers for tenex-edge identities.
//!
//! User-facing session identity is **`sessionCode-agent`** (for example
//! `willow-echo-042-codex`). Backend-qualified agent references (`agent@backend-label`)
//! remain an operator/backend selection syntax for invite and lookup
//! paths, not the public session handle.
//!
//! Rules that hold everywhere:
//!   - Agent session handles are rendered with [`session_handle`].
//!   - Backend labels are config.json `backendName` values and are preserved
//!     exactly after trimming. They are not DNS hostnames and are not slugified.
//!   - Raw session ids are internal correlation handles. A friendly session
//!     codename appears before the agent slug in `sessionCode-agent`.
//!
//! Every renderer formats through this module; every input is classified via
//! [`parse_ref`] or normalized through the session-handle helpers.

use nostr_sdk::prelude::{PublicKey, ToBech32};

pub fn normalize_pubkey(value: &str) -> Option<String> {
    PublicKey::parse(value.trim()).ok().map(|pk| pk.to_hex())
}

pub fn npub(pubkey: &str) -> Option<String> {
    PublicKey::parse(pubkey).ok()?.to_bech32().ok()
}

/// Canonical user-facing session handle: `sessionCode-agentSlug`.
pub fn session_handle(agent_slug: &str, session: &str) -> String {
    let agent_slug = agent_slug.trim();
    let session = session.trim();
    if session.is_empty() {
        return agent_slug.to_string();
    }
    if agent_slug.is_empty() {
        return session.to_string();
    }
    if session.ends_with(&format!("-{agent_slug}")) {
        return session.to_string();
    }
    format!("{session}-{agent_slug}")
}

/// Convert a kind:0 `name` plus tags into the canonical session handle.
pub fn session_handle_from_profile_name(name: &str, agent_slug: &str) -> String {
    let name = name.trim();
    if agent_slug.trim().is_empty() || name == agent_slug.trim() || normalize_pubkey(name).is_some()
    {
        name.to_string()
    } else {
        session_handle(agent_slug, name)
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
/// already-normalized session handle; callers that provide a bare slug plus
/// host degrade through the backend-qualified form.
pub fn session_label(session_id: &str, slug: &str, host: &str) -> String {
    if slug.is_empty() {
        session_id.to_string()
    } else {
        let _ = host;
        slug.to_string()
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
    /// Anything else: an opaque current handle or a bare local agent label.
    /// The daemon resolver consults the authoritative store for the surface.
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

#[cfg(test)]
mod tests;
