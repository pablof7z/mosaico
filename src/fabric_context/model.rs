//! Canonical agent fabric document.
//!
//! Capture and assembly decide which nodes exist. The sole agent XML renderer
//! serializes this model without knowing the cursor, caller, or surface.

#[derive(Clone, Default, PartialEq)]
pub(crate) struct FabricView {
    pub(in crate::fabric_context) self_row: Option<SelfRow>,
    /// `Some` means the canonical `<hosts>` node is present, including when its
    /// full-state value is empty. Delta assembly uses `None` when it is unchanged.
    pub(in crate::fabric_context) hosts: Option<Vec<HostRow>>,
    /// `Some` means the canonical `<workspaces>` node is present. Each workspace
    /// contains only the channel nodes selected by assembly for this cursor.
    pub(in crate::fabric_context) workspaces: Option<Vec<WorkspaceView>>,
    pub(in crate::fabric_context) important: Vec<ImportantRow>,
    pub(in crate::fabric_context) reactions: Vec<ReactionRow>,
    pub(in crate::fabric_context) reactions_omitted: usize,
    pub(in crate::fabric_context) warnings: Vec<WarningRow>,
    pub(in crate::fabric_context) notices: Vec<NoticeRow>,
}

impl FabricView {
    /// Self identity is framing, not activity. A view containing only `<self>`
    /// remains suppressible on a quiet unforced hook turn.
    pub(crate) fn is_empty(&self) -> bool {
        self.hosts.is_none()
            && self.workspaces.is_none()
            && self.important.is_empty()
            && self.reactions.is_empty()
            && self.warnings.is_empty()
            && self.notices.is_empty()
    }

    pub(in crate::fabric_context) fn has_activity(&self) -> bool {
        self.hosts.as_ref().is_some_and(|rows| !rows.is_empty())
            || self
                .workspaces
                .as_ref()
                .is_some_and(|rows| !rows.is_empty())
            || !self.important.is_empty()
            || !self.reactions.is_empty()
            || !self.warnings.is_empty()
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct SelfRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) host: String,
    pub(in crate::fabric_context) headless: bool,
    pub(in crate::fabric_context) title: String,
    pub(in crate::fabric_context) hint: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct HostRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) agents: Vec<AgentRow>,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct AgentRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, Default, PartialEq)]
pub(in crate::fabric_context) struct WorkspaceView {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) hosts: Vec<String>,
    pub(in crate::fabric_context) root: Option<ChannelBlock>,
    /// Channels whose ancestors are unavailable remain explicit top-level rows
    /// rather than being silently dropped.
    pub(in crate::fabric_context) channels: Vec<ChannelBlock>,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ChannelBlock {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) member_count: Option<usize>,
    pub(in crate::fabric_context) last_active: Option<String>,
    pub(in crate::fabric_context) members: Vec<MemberRow>,
    pub(in crate::fabric_context) presence: Vec<PresenceRow>,
    pub(in crate::fabric_context) children: Vec<ChannelBlock>,
    pub(in crate::fabric_context) messages: Vec<MessageRow>,
    pub(in crate::fabric_context) omitted: usize,
}

impl ChannelBlock {
    pub(in crate::fabric_context) fn is_compact(&self) -> bool {
        self.members.is_empty()
            && self.presence.is_empty()
            && self.children.is_empty()
            && self.messages.is_empty()
            && self.omitted == 0
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(in crate::fabric_context) enum MemberKind {
    Agent,
    Human,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct MemberRow {
    pub(in crate::fabric_context) kind: MemberKind,
    pub(in crate::fabric_context) name: String,
    /// `Some` only for a member with a live heartbeat. A member surfaced purely
    /// from message activity has no lifecycle we can vouch for, so it carries no
    /// state label — just `since`.
    pub(in crate::fabric_context) state: Option<crate::session_state::SessionState>,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) since: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct PresenceRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) state: crate::session_state::SessionState,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) since: String,
    pub(in crate::fabric_context) native_failure: Option<NativeFailureRow>,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct NativeFailureRow {
    pub(in crate::fabric_context) outcome: String,
    pub(in crate::fabric_context) message: String,
    pub(in crate::fabric_context) since: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct MessageRow {
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) channel_ref: String,
    pub(in crate::fabric_context) from: String,
    pub(in crate::fabric_context) recipients: Vec<String>,
    pub(in crate::fabric_context) age: String,
    pub(in crate::fabric_context) body: String,
    pub(in crate::fabric_context) mention: bool,
    pub(in crate::fabric_context) truncated: bool,
    pub(in crate::fabric_context) needs_reply_nudge: bool,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ImportantRow {
    pub(in crate::fabric_context) channel_ref: String,
    pub(in crate::fabric_context) message_id: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ReactionRow {
    pub(in crate::fabric_context) reactors: Vec<String>,
    pub(in crate::fabric_context) emoji: String,
    pub(in crate::fabric_context) target_snippet: String,
    pub(in crate::fabric_context) age: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct WarningRow {
    pub(in crate::fabric_context) text: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) enum NoticeRow {
    NoNewActivity { workspace: String },
}
