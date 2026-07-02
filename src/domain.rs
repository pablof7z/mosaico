//! Pure domain model. No Nostr, no kinds, no tags, no wire format.
//!
//! The litmus test for the codec seam (M1 §3): this module must not name
//! concrete Nostr kinds, tags, or wire-protocol concepts. Everything here is
//! what tenex-edge *means*, never how it travels.

use crate::util::SessionId;

/// Liveness TTL: a status is "live" while its heartbeat is fresher than this.
/// The daemon re-arms the kind:30315 NIP-40 `expiration` to `now + STATUS_TTL_SECS`
/// on every heartbeat, so a stopped session's event expires off the relay ~this
/// long after its last beat. 90s matches the `who` peer-freshness window so local
/// and peer liveness use one number.
pub const STATUS_TTL_SECS: u64 = 90;

/// Heartbeat cadence. 3x re-arm margin under `STATUS_TTL_SECS` (no flicker).
pub const HEARTBEAT_SECS: u64 = 30;

/// The lifecycle of a session aggregate. PURE marker (never on the wire — there
/// are no tombstone events): a stopped session is detected by its status event
/// expiring, not by an `ended`/`superseded` signal. Stored on `sessions` so
/// readers and `derive_status` can suppress a finished session locally before its
/// relay event ages out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lifecycle {
    /// Running or idle-but-resumable; the normal state.
    #[default]
    Active,
    /// The session finished (clean exit / session-end). Title retained.
    Ended,
    /// A newer logical session took this one's pane/pid slot.
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
/// to (p-tagged), so a recipient can decide whether to authorize it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub agent: AgentRef,
    pub host: String,
    /// Owner pubkeys this agent claims (the human's whitelisted pubkeys).
    pub owners: Vec<String>,
    /// True when published by the tenex-edge backend process itself (not an AI
    /// agent). Encoded as a `["backend"]` tag on the wire; used to suppress
    /// backend identities from agent-facing context injections.
    pub is_backend: bool,
}

/// A durable, append-only line of narrative: what the agent is doing / did.
/// Used for social Activity notes (kind:1 without p tag).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    pub agent: AgentRef,
    pub project: String,
    pub text: String,
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
    /// Owner pubkeys the proposal is surfaced to.
    pub audience: Vec<String>,
}

/// The agent's complete live state for one local session. On the wire, NIP-29
/// status is replaceable by `(author pubkey, session_id)`, and targets every
/// channel the session is currently present in via repeated `h` tags. From this
/// one value a reader knows everything: who/where (agent, channels, host,
/// session, rel_cwd), what the
/// session is about (the persistent `title`), what it is doing *right now* (the
/// live `activity`), and whether it is mid-turn (`busy`). It is replaceable per
/// session, so each session keeps its own title even while idle — and after it
/// exits.
///
/// Liveness = freshness of THIS event. The daemon re-arms `expires_at` to
/// `now + STATUS_TTL_SECS` on every heartbeat; the codec turns `Some(ts)` into a
/// NIP-40 `["expiration", ts]` tag. Beats stop → event expires → reads as dead.
/// `expires_at == None` publishes without an expiration (used only in tests /
/// non-heartbeat contexts).
///
/// PROVIDER STATUS API (implemented by the daemon's provider, NOT in this pure
/// module — specified here so the codec/provider/drainer agents bind to it):
///
/// ```ignore
/// impl crate::fabric::provider::Nip29Provider {
///     /// Encode `status` to kind:30315 (NIP-40 expiration when `expires_at` is
///     /// Some), sign with `keys`, and return only after checked relay
///     /// acceptance. Returns the native event id.
///     pub async fn set_status(
///         &self,
///         status: &crate::domain::Status,
///         keys: &nostr_sdk::prelude::Keys,
///     ) -> anyhow::Result<nostr_sdk::prelude::EventId>;
/// }
/// ```
///
/// The per-session engine encodes and signs a kind:30315 status into the
/// durable `outbox`. The daemon drainer publishes that signed JSON with a
/// checked relay verdict, then marks the row `published` only after acceptance.
/// The heartbeat publisher that bypasses the outbox uses `set_status`, which
/// applies the same checked verdict before refreshing the local status cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub agent: AgentRef,
    /// NIP-29 channels this session is currently present in. Encoded as one
    /// `h` tag per channel; materializers fan this single session status out to
    /// per-channel read rows.
    pub channels: Vec<String>,
    /// The session this status belongs to. Encoded as the kind:30315 `d` tag.
    pub session_id: SessionId,
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
    /// Whether the session is mid-turn (busy). Decoupled from `title` so an idle
    /// session keeps showing its title with a separate idle marker.
    pub busy: bool,
    /// Project-relative working directory (e.g. `worktree1`, `sub/dir`, `.`).
    /// PUBLIC kind → never the absolute `$HOME/...` path (privacy). Lets a `who`
    /// reflect where the agent is working.
    pub rel_cwd: String,
    /// NIP-40 expiration (unix secs). `Some(now + STATUS_TTL_SECS)` on every
    /// heartbeat re-arm; `None` publishes without an expiration tag. Liveness IS
    /// the freshness of this event.
    pub expires_at: Option<u64>,
}

impl Status {
    pub fn primary_channel(&self) -> Option<&str> {
        self.channels.first().map(String::as_str)
    }

    /// Idle = not mid-turn. The `title` persists across idle turns, so idle is
    /// no longer "empty title".
    pub fn is_idle(&self) -> bool {
        !self.busy
    }
}

/// A NIP-29 project chat line. On the wire this is a NIP-C7 `kind:9` event
/// scoped to the project group by its `h` tag. It is ambient project context;
/// live sessions see it going forward only. Chat fans out to every alive project
/// session by pubkey + channel membership; session ids are for lifecycle/status,
/// not chat addressing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub from: AgentRef,
    pub project: String,
    pub body: String,
    /// Optional pubkey for the @-mentioned agent, carried as a Nostr `p` tag.
    pub mentioned_pubkey: Option<String>,
}

/// The closed set of things that travel on the fabric. A codec encodes each of
/// these to a wire envelope and decodes wire envelopes back into these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Profile(Profile),
    Activity(Activity),
    Status(Status),
    ChatMessage(ChatMessage),
    Proposal(Proposal),
}

impl DomainEvent {
    /// The NIP-29 group h-tag this event targets, if any.
    /// `Profile` (kind:0) is not scoped to a group and returns `None`.
    pub fn channel(&self) -> Option<&str> {
        match self {
            DomainEvent::Profile(_) => None,
            DomainEvent::Activity(a) => Some(&a.project),
            DomainEvent::Status(s) => s.primary_channel(),
            DomainEvent::ChatMessage(m) => Some(&m.project),
            DomainEvent::Proposal(p) => Some(&p.project),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_idle_detection() {
        let agent = AgentRef::new("pk", "coder");
        // A title is retained while idle: idle tracks `busy`, not the title.
        let idle = Status {
            agent: agent.clone(),
            channels: vec!["p".into()],
            session_id: "s1".into(),
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            busy: false,
            rel_cwd: String::new(),
            expires_at: None,
        };
        let busy = Status {
            agent,
            channels: vec!["p".into()],
            session_id: "s1".into(),
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            busy: true,
            rel_cwd: String::new(),
            expires_at: None,
        };
        assert!(idle.is_idle());
        assert!(!busy.is_idle());
    }
}
