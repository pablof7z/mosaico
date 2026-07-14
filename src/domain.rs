//! Pure domain model. No concrete fabric codes, labels, or serialization
//! format.
//!
//! The litmus test for the provider seam: everything here is what tenex-edge
//! *means*, never how it travels.

/// Liveness TTL: a status is "live" while its heartbeat is fresher than this.
/// The daemon refreshes the provider-native expiry to `now + STATUS_TTL_SECS` on
/// every heartbeat, so a stopped session disappears from live views roughly this
/// long after its last beat. 90s matches the `who` peer-freshness window so
/// local and peer liveness use one number.
pub const STATUS_TTL_SECS: u64 = 90;

/// Heartbeat cadence. 3x re-arm margin under `STATUS_TTL_SECS` (no flicker).
pub const HEARTBEAT_SECS: u64 = 30;

/// The lifecycle of a session aggregate. Stored on `sessions` so readers and
/// `derive_status` can suppress a finished session locally before remote
/// liveness expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lifecycle {
    /// Running or idle-but-resumable; the normal state.
    #[default]
    Active,
    /// The session finished (clean exit / session-end). Title retained.
    Ended,
    /// A newer logical session took this one's PTY/pid slot.
    Superseded,
}

impl Lifecycle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Lifecycle::Active => "active",
            Lifecycle::Ended => "ended",
            Lifecycle::Superseded => "superseded",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "ended" => Lifecycle::Ended,
            "superseded" => Lifecycle::Superseded,
            _ => Lifecycle::Active,
        }
    }
    /// Whether this session may still be reported live (only `Active`).
    pub fn is_active(&self) -> bool {
        matches!(self, Lifecycle::Active)
    }
}

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
/// to so a recipient can decide whether to authorize it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub agent: AgentRef,
    /// Stable harness/role slug for the agent behind this session identity.
    /// `agent.slug` is the published display handle, usually `sessionCode-agent`.
    pub agent_slug: String,
    pub host: String,
    /// Top-level workspace channel this live agent session is working from.
    /// Empty for backend and retired profiles.
    pub workspace: String,
    /// Owner pubkeys this agent claims (the human's whitelisted pubkeys).
    pub owners: Vec<String>,
    /// True when published by the tenex-edge backend process itself (not an AI
    /// agent). Used to suppress backend identities from agent-facing context
    /// injections.
    pub is_backend: bool,
    /// `(slug, description)` for each agent this backend manages. Only populated
    /// on the backend profile (empty for agent sessions); serialized as
    /// `["agent", slug, description]` tags so clients can offer an add-agent
    /// picker. `slug` is command-compatible with `add <slug>`; `description` is
    /// the agent's `effective_byline`, matching the kind:30555 roster.
    pub agents: Vec<(String, String)>,
}

impl Profile {
    pub fn agent(
        agent: AgentRef,
        agent_slug: impl Into<String>,
        host: impl Into<String>,
        owners: Vec<String>,
    ) -> Self {
        Self {
            agent,
            agent_slug: agent_slug.into(),
            host: host.into(),
            workspace: String::new(),
            owners,
            is_backend: false,
            agents: Vec::new(),
        }
    }

    pub fn backend(agent: AgentRef, host: impl Into<String>, owners: Vec<String>) -> Self {
        Self {
            agent,
            agent_slug: String::new(),
            host: host.into(),
            workspace: String::new(),
            owners,
            is_backend: true,
            agents: Vec::new(),
        }
    }

    pub fn backend_named(
        pubkey: impl Into<String>,
        name: impl Into<String>,
        host: impl Into<String>,
        owners: Vec<String>,
    ) -> Self {
        Self::backend(AgentRef::new(pubkey, name), host, owners)
    }

    /// Attach the managed-agent roster `(slug, description)` advertised on the
    /// backend kind:0. No-op semantics for agent sessions — callers only set
    /// this on backend profiles.
    pub fn with_agents(mut self, agents: Vec<(String, String)>) -> Self {
        self.agents = agents;
        self
    }

    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = workspace.into();
        self
    }
}

/// A durable, append-only line of narrative: what the agent is doing / did.
/// Used for social activity notes that are not inbox-routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub agent: AgentRef,
    pub channel: String,
    pub text: String,
}

/// The agent's complete live state. From this one value a reader knows
/// everything: who/where (agent, channels, host, rel_cwd),
/// what the session is about (the persistent `title`), what it is doing *right
/// now* (the live `activity`), and its canonical user-facing state. It is scoped
/// per authoritative agent pubkey, so each agent keeps its title even while idle.
///
/// Liveness = freshness of this state. The daemon refreshes `expires_at` on
/// every heartbeat. Beats stop, the provider-native expiry passes, and readers
/// treat the session as dead. `expires_at == None` publishes without an expiry
/// (used only in tests / non-heartbeat contexts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub agent: AgentRef,
    /// Channels this session is currently present in. Materializers fan this
    /// single session status out to per-channel read rows.
    pub channels: Vec<String>,
    /// The machine this session lives on.
    pub host: String,
    /// The session title: a short, stable description of what the session is
    /// about. Retained across idle turns AND after the session exits — a
    /// finished session keeps its title on the fabric. Never cleared.
    pub title: String,
    /// The live activity line: what the agent is doing *right now* (the current
    /// step/mechanics). Distilled alongside `title` in one model call and
    /// refreshed every turn; cleared on idle (only the persistent title
    /// survives). Empty when no live activity is known.
    pub activity: String,
    /// Canonical user-facing state, normalized from host/runtime facts.
    pub state: crate::session_state::SessionState,
    /// Channel-relative working directory (e.g. `worktree1`, `sub/dir`, `.`).
    /// Public status value: never the absolute `$HOME/...` path (privacy). Lets
    /// awareness projections reflect where the agent is working.
    pub rel_cwd: String,
    /// Expiration timestamp (unix secs). `Some(now + STATUS_TTL_SECS)` on every
    /// heartbeat re-arm; `None` publishes without an expiry. Liveness IS the
    /// freshness of this state.
    pub expires_at: Option<u64>,
    /// Dispatch kind:9 event id that caused this session to start, when any.
    /// Encoded as a status `e` tag so requesters can correlate the ACK.
    pub dispatch_event: Option<String>,
}

impl Status {
    pub fn primary_channel(&self) -> Option<&str> {
        self.channels.first().map(String::as_str)
    }

    /// The `title` persists across idle turns, so idle is not "empty title".
    pub fn is_idle(&self) -> bool {
        self.state == crate::session_state::SessionState::Idle
    }
}

/// A channel chat line. It is ambient channel context; live sessions see it
/// going forward only. Chat fans out to every alive channel session by pubkey +
/// channel membership; private runtime ids never address chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub from: AgentRef,
    pub channel: String,
    pub body: String,
    /// Pubkeys explicitly tagged by the sender.
    pub mentioned_pubkeys: Vec<String>,
}

/// Largest reaction payload we accept. Comfortably fits any single emoji,
/// including multi-codepoint ZWJ sequences, while bounding awareness token cost
/// and denying natural-language content a foothold in the turn-start context.
pub const MAX_REACTION_EMOJI_BYTES: usize = 16;

/// A non-disruptive acknowledgement of a specific message (NIP-25 kind:7). It is
/// passive awareness only: a reaction never routes to the inbox, never wakes an
/// idle agent, and never injects mid-turn. The target agent sees it as a compact
/// delta at its next turn-start hook. `content` is the emoji (or `+`/`-`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub reactor: AgentRef,
    pub channel: String,
    /// The reacted-to message's native event id (the `e` tag).
    pub target_event_id: String,
    /// The reaction emoji, or the NIP-25 `+`/`-` shorthand.
    pub emoji: String,
}

impl Reaction {
    /// The single trust-boundary predicate for reaction payloads: trimmed
    /// non-empty, at most [`MAX_REACTION_EMOJI_BYTES`] bytes, and free of
    /// whitespace/control characters. Applied to BOTH locally originated reactions
    /// (the RPC) and inbound relay kind:7 events (the wire decoder), so an
    /// adversarial peer cannot smuggle large or multi-line natural-language content
    /// into a target agent's turn-start awareness via a kind:7 `content`.
    pub fn emoji_is_valid(emoji: &str) -> bool {
        let e = emoji.trim();
        !e.is_empty()
            && e.len() <= MAX_REACTION_EMOJI_BYTES
            && !e.chars().any(|c| c.is_control() || c.is_whitespace())
    }
}

/// The closed set of things that travel on the fabric. A provider maps these to
/// and from its native representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Profile(Profile),
    Activity(Activity),
    Status(Status),
    ChatMessage(ChatMessage),
    Reaction(Reaction),
}

impl DomainEvent {
    /// The channel this event targets, if any.
    /// `Profile` is not scoped to a channel and returns `None`.
    pub fn channel(&self) -> Option<&str> {
        match self {
            DomainEvent::Profile(_) => None,
            DomainEvent::Activity(a) => Some(&a.channel),
            DomainEvent::Status(s) => s.primary_channel(),
            DomainEvent::ChatMessage(m) => Some(&m.channel),
            DomainEvent::Reaction(r) => Some(&r.channel),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_idle_detection() {
        let agent = AgentRef::new("pk", "coder");
        // A title is retained while idle: state, not title content, is canonical.
        let idle = Status {
            agent: agent.clone(),
            channels: vec!["p".into()],
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            rel_cwd: String::new(),
            expires_at: None,
            dispatch_event: None,
        };
        let working = Status {
            agent,
            channels: vec!["p".into()],
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            state: crate::session_state::SessionState::Working,
            rel_cwd: String::new(),
            expires_at: None,
            dispatch_event: None,
        };
        assert!(idle.is_idle());
        assert!(!working.is_idle());
    }
}
