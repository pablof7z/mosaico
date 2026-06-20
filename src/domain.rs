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
/// expiring, not by an `ended`/`superseded` signal. Stored on `session_state` so
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

/// The agent's complete live state for ONE session — the single self-contained
/// per-session signal on the fabric. From this one value a reader knows
/// everything: who/where (agent, project, host, session, rel_cwd), what the
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
/// impl crate::fabric::provider::Kind1Nip29Provider {
///     /// Encode `status` to kind:30315 (NIP-40 expiration when `expires_at` is
///     /// Some), sign with `keys`, publish. Returns the native event id.
///     pub async fn set_status(
///         &self,
///         status: &crate::domain::Status,
///         keys: &nostr_sdk::prelude::Keys,
///     ) -> anyhow::Result<nostr_sdk::prelude::EventId>;
/// }
/// ```
///
/// The daemon's status-outbox drainer builds a `Status` from each pending
/// `SessionSnapshot` (setting `expires_at = now + STATUS_TTL_SECS`), calls
/// `set_status`, then marks the `status_outbox` row published with the returned
/// id. The per-heartbeat liveness re-arm re-publishes the latest snapshot the
/// same way (it does NOT enqueue an outbox row). Nothing above the provider
/// builds a kind:30315 event directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub agent: AgentRef,
    pub project: String,
    /// The session this status belongs to. Every status is per-session — this is
    /// what makes the wire event replaceable per `(project, session)`.
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
    /// The session's conversation thread root: the event id of the FIRST user
    /// prompt that opened this session (stored locally as
    /// `session_state.thread_root_event_id`). Published as a NIP-10
    /// `["e", root, "", "root"]` tag so a reader can map this session to its
    /// conversation — the kind:1 prompts/replies share no other key with the
    /// kind:30315. `None` until the first prompt arrives.
    pub thread_root_id: Option<String>,
}

impl Status {
    /// Idle = not mid-turn. The `title` persists across idle turns, so idle is
    /// no longer "empty title".
    pub fn is_idle(&self) -> bool {
        !self.busy
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

/// A NIP-29 project chat line. On the wire this is a NIP-C7 `kind:9` event
/// scoped to the project group by its `h` tag. It is ambient project context,
/// not a durable direct inbox item; live sessions see it going forward only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub from: AgentRef,
    pub project: String,
    pub body: String,
    /// The sender's session id, when known.
    pub from_session: Option<SessionId>,
    /// Optional session explicitly mentioned in the chat line.
    pub mentioned_session: Option<SessionId>,
    /// Optional pubkey for the mentioned session, carried as a Nostr `p` tag.
    pub mentioned_pubkey: Option<String>,
}

/// The closed set of things that travel on the fabric. A codec encodes each of
/// these to a wire envelope and decodes wire envelopes back into these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    Profile(Profile),
    Activity(Activity),
    Status(Status),
    Mention(Mention),
    ChatMessage(ChatMessage),
    TurnReply(TurnReply),
    Proposal(Proposal),
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
            project: "p".into(),
            session_id: "s1".into(),
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            busy: false,
            rel_cwd: String::new(),
            expires_at: None,
            thread_root_id: None,
        };
        let busy = Status {
            agent,
            project: "p".into(),
            session_id: "s1".into(),
            host: "laptop".into(),
            title: "fixing auth".into(),
            activity: String::new(),
            busy: true,
            rel_cwd: String::new(),
            expires_at: None,
            thread_root_id: None,
        };
        assert!(idle.is_idle());
        assert!(!busy.is_idle());
    }
}
