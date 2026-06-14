//! Pure domain model. No Nostr, no kinds, no tags, no wire format.
//!
//! The litmus test for the codec seam (M1 §3): this module must not name
//! concrete Nostr kinds, tags, or wire-protocol concepts. Everything here is
//! what tenex-edge *means*, never how it travels.

use crate::util::SessionId;

/// A reference to an agent: its sovereign pubkey and the slug it goes by.
/// Identity is `(agent, machine)` — the same tool on another machine is a
/// different agent with a different pubkey (M1 §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRef {
    pub pubkey: String, // hex
    pub slug: String,
}

impl AgentRef {
    pub fn new(pubkey: impl Into<String>, slug: impl Into<String>) -> Self {
        Self {
            pubkey: pubkey.into(),
            slug: slug.into(),
        }
    }
}

/// The agent's published identity card. Resolves `pubkey -> slug`, tells a peer
/// which machine the agent lives on, and declares the human owner(s) it belongs
/// to (p-tagged), so a recipient can decide whether to authorize it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub agent: AgentRef,
    pub host: String,
    /// Owner pubkeys this agent claims (the human's whitelisted pubkeys).
    pub owners: Vec<String>,
}

/// "I am alive, on this project, in this session." The liveness signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    pub agent: AgentRef,
    pub project: String,
    pub session_id: SessionId,
    pub host: String,
    /// Project-relative working directory (e.g. `worktree1`, `sub/dir`, `.`).
    /// PUBLIC kind → never the absolute `$HOME/...` path (privacy).
    pub rel_cwd: String,
    /// Pubkeys this presence is addressed to (the operator's whitelist).
    pub audience: Vec<String>,
    /// Absolute unix seconds after which this heartbeat should be ignored.
    pub expires_at: u64,
}

/// A durable, append-only line of narrative: what the agent is doing / did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub agent: AgentRef,
    pub project: String,
    pub text: String,
}

/// The agent's completed reply to a conversation turn, published at stop-hook
/// time as a NIP-10 threaded kind:1. Carries the response text and two e-tags
/// so any Nostr client can reconstruct the conversation thread:
///   ["e", root_event_id,  "", "root"]  — the first message in the session thread
///   ["e", reply_event_id, "", "reply"] — the user prompt that triggered this turn
///
/// For published artifacts (e.g. kind:30023 long-form articles) generated during
/// a session, the same `root_event_id` should be e-tagged so the artifact can be
/// traced back to the conversation that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReply {
    pub agent: AgentRef,
    pub project: String,
    pub body: String,
    /// Event ID of the first message in this session's conversation thread.
    pub root_event_id: String,
    /// Event ID of the user prompt that triggered this turn.
    pub reply_event_id: String,
}

/// A long-form proposal authored by an agent (rendered as an article by any
/// Nostr client). Addressable: republishing with the same `d` supersedes the
/// prior revision at the same (author, d) address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub agent: AgentRef,
    pub project: String,
    pub title: String,
    pub body: String,
    /// Stable addressable identifier; reuse to publish a superseding revision.
    pub d: String,
    /// Authoring session, when one is live.
    pub session_id: Option<SessionId>,
    /// Owner pubkeys the proposal is surfaced to.
    pub audience: Vec<String>,
    /// Native key of the conversation thread root this proposal belongs to.
    pub thread_root_key: Option<String>,
}

/// The agent's live, replaceable status for a project. `text` is a PERSISTENT
/// session title (what the session is about) that survives idle turns; `active`
/// is the separate "mid-turn" flag. Idle = `!active`, independent of the title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub agent: AgentRef,
    pub project: String,
    /// Session this status belongs to. `None` is legacy agent-level status from
    /// older peers; local runtime publishes session-scoped status.
    pub session_id: Option<SessionId>,
    /// The session title: a short, stable description of what the session is
    /// about. Retained across idle turns; only cleared when the session exits.
    pub text: String,
    /// The live activity line: what the agent is doing *right now* (the current
    /// step/mechanics). Distilled alongside `text` in one model call and refreshed
    /// every turn; cleared on idle (only the persistent title survives). Empty
    /// when no live activity is known.
    pub activity: String,
    /// Whether the session is mid-turn (busy). Decoupled from `text` so an idle
    /// session keeps showing its title with a separate idle marker.
    pub active: bool,
    /// Project-relative working directory (see `Presence::rel_cwd`). Carried on
    /// status too so a mid-turn `who` reflects where the agent is working.
    pub rel_cwd: String,
    /// Absolute unix seconds after which this status should be considered
    /// stale (crash safety). `None` = no expiry.
    pub expires_at: Option<u64>,
}

impl Status {
    /// Idle = not mid-turn. The title (`text`) persists across idle turns, so
    /// idle is no longer "empty text".
    pub fn is_idle(&self) -> bool {
        !self.active
    }
}

/// A directed message from one agent to another, optionally pinned to a
/// specific session of the recipient (M1 §7 routing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mention {
    pub from: AgentRef,
    pub to_pubkey: String,
    pub project: String,
    pub body: String,
    /// When `Some`, only the recipient's matching session should surface it.
    pub target_session: Option<SessionId>,
    /// The SENDER's session id, when known. Lets the recipient reply to the exact
    /// sibling session that wrote this (sessions of one agent share a pubkey, so
    /// the author key alone can't disambiguate them). `None` for old peers.
    pub from_session: Option<SessionId>,
    /// Envelope metadata: subject + the sender's workspace snapshot at send time,
    /// plus the event this is a reply to. Rendered as an email-like header on the
    /// receiving side (see `cli::messaging::format_envelope`).
    pub meta: MentionMeta,
}

/// The email-like envelope a `Mention` carries beyond its body: a subject and a
/// snapshot of the sender's workspace (git branch/commit/dirty, host) captured at
/// send time, plus the original event id when this mention is a reply. All fields
/// default to empty/none for old peers that don't populate them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MentionMeta {
    /// One-line subject ("what this is about"). Empty when unset.
    pub subject: String,
    /// Sender's current git branch (e.g. `features/oauth`). Empty outside a repo.
    pub branch: String,
    /// Sender's short commit hash (e.g. `a1b2c3d`). Empty outside a repo.
    pub commit: String,
    /// Count of dirty, non-gitignored files in the sender's working tree.
    pub dirty: u32,
    /// Sender's host label. The receiver compares it to its own host to decide
    /// whether to annotate the sender as `[remote: <host>]`.
    pub host: String,
    /// When `Some`, the event id this mention replies to (NIP-10 `e` reply tag).
    pub reply_to_event_id: Option<String>,
}

/// The closed set of things that travel on the fabric. A codec encodes each of
/// these to a wire envelope and decodes wire envelopes back into these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Profile(Profile),
    Presence(Presence),
    Activity(Activity),
    Status(Status),
    Mention(Mention),
    TurnReply(TurnReply),
    Proposal(Proposal),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_idle_detection() {
        let agent = AgentRef::new("pk", "coder");
        // A title is retained while idle: idle tracks `active`, not the text.
        let idle = Status {
            agent: agent.clone(),
            project: "p".into(),
            session_id: None,
            text: "fixing auth".into(),
            activity: String::new(),
            active: false,
            rel_cwd: String::new(),
            expires_at: None,
        };
        let busy = Status {
            agent,
            project: "p".into(),
            session_id: None,
            text: "fixing auth".into(),
            activity: String::new(),
            active: true,
            rel_cwd: String::new(),
            expires_at: Some(10),
        };
        assert!(idle.is_idle());
        assert!(!busy.is_idle());
    }
}
