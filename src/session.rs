//! Pure session-host policy shared by the daemon and harness adapters.
//!
//! Session identity does not live here: the signing pubkey is authoritative,
//! while harness-native ids, resume tokens, endpoints, and PIDs are typed local
//! locators. This module only owns the harness vocabulary and the deterministic
//! decision for placing a newly launched runtime in a channel.

pub use crate::domain::STATUS_TTL_SECS;

/// Which agent harness produced an observation. The string form is persisted
/// with typed runtime locators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    Opencode,
    Grok,
    Unknown,
}

impl Harness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::Grok => "grok",
            Harness::Unknown => "unknown",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "claude-code" | "claude" => Harness::ClaudeCode,
            "codex" => Harness::Codex,
            "opencode" => Harness::Opencode,
            "grok" => Harness::Grok,
            _ => Harness::Unknown,
        }
    }

    /// User-facing agent slug for launching a harness with its default profile.
    pub fn agent_slug(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::Grok => "grok",
            Harness::Unknown => "unknown",
        }
    }
}

/// Where a newly born session's fabric events land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomDecision {
    /// Human-initiated session: mint a leaf under `parent`.
    Mint { parent: String },
    /// Orchestration-spawned session: join the provided group as-is.
    UseExisting { group: String },
}

/// Decide whether session birth mints a per-session room.
pub fn decide_session_room(
    group: Option<&str>,
    work_root: &str,
    per_session_rooms: bool,
) -> RoomDecision {
    match group {
        Some(g) if !g.is_empty() => RoomDecision::UseExisting {
            group: g.to_string(),
        },
        _ if per_session_rooms => RoomDecision::Mint {
            parent: work_root.to_string(),
        },
        _ => RoomDecision::UseExisting {
            group: work_root.to_string(),
        },
    }
}

#[cfg(test)]
mod tests;
