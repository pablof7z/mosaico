// The rendered fabric-context view. `Clone + PartialEq` so it can travel through
// a Trellis materialized output as a typed payload (change-detected by the graph)
// and be compared byte-for-byte in tests. The type is `pub(crate)` so the
// reconcile spine can name it as an output payload; its fields stay module-private
// so only `build`/`capture` construct it and `render`/`human_render` read it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct FabricView {
    pub(in crate::fabric_context) self_row: Option<SelfRow>,
    pub(in crate::fabric_context) workspace: WorkspaceRow,
    pub(in crate::fabric_context) agents: Vec<AgentRow>,
    pub(in crate::fabric_context) channels: Vec<ChannelBlock>,
    pub(in crate::fabric_context) unjoined: Vec<UnjoinedChannelRow>,
    pub(in crate::fabric_context) important: Vec<ImportantRow>,
    pub(in crate::fabric_context) warnings: Vec<WarningRow>,
    /// True when this view was built in delta mode (cursor > 0): it carries only
    /// what changed since the caller's last snapshot, not the full current state.
    /// The renderer uses it to explain a quiet result rather than emitting a bare
    /// empty workspace block that reads as "everything disappeared".
    pub(in crate::fabric_context) incremental: bool,
}

impl FabricView {
    /// Whether the view carries nothing renderable — the daemon suppresses an
    /// empty snapshot unless the caller forced it.
    pub(crate) fn is_empty(&self) -> bool {
        self.channels.is_empty()
            && self.agents.is_empty()
            && self.important.is_empty()
            && self.warnings.is_empty()
    }

    /// A forced delta snapshot that surfaced nothing new. Rendered as an explicit
    /// "nothing changed" note (see `render_no_new_activity`) instead of an empty
    /// `<workspace>` skeleton, so a quiet fabric never looks like data loss. Unjoined
    /// channels and warnings count as content worth showing, so their presence
    /// takes the normal render path.
    pub(in crate::fabric_context) fn is_quiet_delta(&self) -> bool {
        self.incremental
            && self.channels.is_empty()
            && self.agents.is_empty()
            && self.unjoined.is_empty()
            && self.important.is_empty()
            && self.warnings.is_empty()
    }
}

#[derive(Clone, Default, PartialEq)]
pub(in crate::fabric_context) struct WorkspaceRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct SelfRow {
    pub(in crate::fabric_context) agent: String,
    pub(in crate::fabric_context) agent_slug: String,
    pub(in crate::fabric_context) host: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct AgentRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ChannelBlock {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) workspace: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) members: Vec<MemberRow>,
    pub(in crate::fabric_context) presence: Vec<PresenceRow>,
    pub(in crate::fabric_context) subchannels: Vec<ChannelSummaryRow>,
    pub(in crate::fabric_context) messages: Vec<MessageRow>,
    pub(in crate::fabric_context) omitted: usize,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct MemberRow {
    /// `@agent/session` for a member whose session identity is known, else the
    /// slug/npub `pubkey_ref` fallback (human operators, offline sessions).
    pub(in crate::fabric_context) reference: String,
    /// Stable underlying agent kind from the session row or kind:0 profile tag.
    pub(in crate::fabric_context) agent_slug: String,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) seen: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct PresenceRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) seen: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ChannelSummaryRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct MessageRow {
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) from: String,
    pub(in crate::fabric_context) recipients: Vec<String>,
    pub(in crate::fabric_context) age: String,
    pub(in crate::fabric_context) body: String,
    pub(in crate::fabric_context) mention: bool,
    pub(in crate::fabric_context) truncated: bool,
}

/// A channel in the workspace this agent has not joined — not a dormant one;
/// joined channels never appear here regardless of how quiet they are.
#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct UnjoinedChannelRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) last_active: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ImportantRow {
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) message_id: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct WarningRow {
    pub(in crate::fabric_context) text: String,
}
