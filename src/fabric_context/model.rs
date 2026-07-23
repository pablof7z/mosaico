// The rendered fabric-context view. `Clone + PartialEq` supports change detection
// and byte-for-byte comparison in tests. Fields stay module-private so only
// capture/assembly construct it and the renderers read it.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct FabricView {
    pub(in crate::fabric_context) self_row: Option<SelfRow>,
    pub(in crate::fabric_context) workspace: WorkspaceRow,
    pub(in crate::fabric_context) agents: Vec<AgentRow>,
    /// Activity owned by the workspace's root channel. Rendered directly on the
    /// workspace so the root is not repeated as a child channel.
    pub(in crate::fabric_context) root: Option<ChannelBlock>,
    pub(in crate::fabric_context) channels: Vec<ChannelBlock>,
    /// Presence deltas from workspace roots other than the current session's.
    /// Chat remains scoped to the session's joined channels.
    pub(in crate::fabric_context) other_workspaces: Vec<WorkspaceActivity>,
    pub(in crate::fabric_context) important: Vec<ImportantRow>,
    /// Reactions on THIS agent's own recent messages, delta-scoped to the cursor
    /// so each reaction surfaces exactly once. Passive awareness only — never a
    /// mention, never an inject.
    pub(in crate::fabric_context) reactions: Vec<ReactionRow>,
    /// Reaction groups elided by the render cap.
    pub(in crate::fabric_context) reactions_omitted: usize,
    pub(in crate::fabric_context) warnings: Vec<WarningRow>,
    /// True when this view was built in delta mode (cursor > 0): it carries only
    /// what changed since the caller's last snapshot, not the full current state.
    /// The renderer uses it to explain a quiet result rather than emitting a bare
    /// empty workspace block that reads as "everything disappeared".
    pub(in crate::fabric_context) incremental: bool,
}

impl FabricView {
    /// Agent hook context omits the capability roster. Removing the rows before
    /// change detection also keeps roster-only relay updates from producing
    /// empty hook frames.
    pub(crate) fn for_agent_context(mut self) -> Self {
        self.agents.clear();
        self
    }

    /// Whether the view carries nothing renderable — the daemon suppresses an
    /// empty snapshot unless the caller forced it.
    pub(crate) fn is_empty(&self) -> bool {
        self.root.is_none()
            && self.channels.is_empty()
            && self.other_workspaces.is_empty()
            && self.agents.is_empty()
            && self.important.is_empty()
            && self.reactions.is_empty()
            && self.warnings.is_empty()
    }

    /// A forced delta snapshot that surfaced nothing new. Rendered as an explicit
    /// "nothing changed" note (see `render_no_new_activity`) instead of an empty
    /// `<workspace>` skeleton, so a quiet fabric never looks like data loss.
    pub(in crate::fabric_context) fn is_quiet_delta(&self) -> bool {
        self.incremental
            && self.root.is_none()
            && self.channels.is_empty()
            && self.other_workspaces.is_empty()
            && self.agents.is_empty()
            && self.important.is_empty()
            && self.reactions.is_empty()
            && self.warnings.is_empty()
    }
}

/// One grouped reaction awareness row: every peer who reacted with the same
/// `emoji` to the same message of yours, collapsed into a single line.
#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ReactionRow {
    /// Resolved display refs of the reactors (deduped, sorted).
    pub(in crate::fabric_context) reactors: Vec<String>,
    pub(in crate::fabric_context) emoji: String,
    /// A short snippet of the reacted-to message body.
    pub(in crate::fabric_context) target_snippet: String,
    /// Relative age of the most recent reaction in the group.
    pub(in crate::fabric_context) age: String,
}

#[derive(Clone, Default, PartialEq)]
pub(in crate::fabric_context) struct WorkspaceActivity {
    pub(in crate::fabric_context) workspace: WorkspaceRow,
    pub(in crate::fabric_context) root: Option<ChannelBlock>,
    pub(in crate::fabric_context) channels: Vec<ChannelBlock>,
}

#[derive(Clone, Default, PartialEq)]
pub(in crate::fabric_context) struct WorkspaceRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct SelfRow {
    pub(in crate::fabric_context) agent: String,
    pub(in crate::fabric_context) agent_slug: String,
    pub(in crate::fabric_context) host: String,
    pub(in crate::fabric_context) title: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct AgentRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
}

/// Agents declared in every workspace view. The all-workspaces renderers can
/// show these once, then emit only each workspace's roster delta.
pub(in crate::fabric_context) fn shared_agents(views: &[FabricView]) -> Vec<AgentRow> {
    let Some((first, rest)) = views.split_first() else {
        return Vec::new();
    };
    first
        .agents
        .iter()
        .filter(|agent| rest.iter().all(|view| view.agents.contains(agent)))
        .cloned()
        .collect()
}

pub(in crate::fabric_context) fn workspace_agents(
    view: &FabricView,
    shared: &[AgentRow],
) -> Vec<AgentRow> {
    view.agents
        .iter()
        .filter(|agent| !shared.contains(agent))
        .cloned()
        .collect()
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ChannelBlock {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) members: Vec<MemberRow>,
    pub(in crate::fabric_context) presence: Vec<PresenceRow>,
    pub(in crate::fabric_context) children: Vec<ChannelBlock>,
    pub(in crate::fabric_context) messages: Vec<MessageRow>,
    pub(in crate::fabric_context) omitted: usize,
}

impl ChannelBlock {
    pub(in crate::fabric_context) fn compact(
        name: String,
        reference: String,
        about: String,
    ) -> Self {
        Self {
            name,
            reference,
            about,
            members: Vec::new(),
            presence: Vec::new(),
            children: Vec::new(),
            messages: Vec::new(),
            omitted: 0,
        }
    }

    pub(in crate::fabric_context) fn is_compact(&self) -> bool {
        self.members.is_empty()
            && self.presence.is_empty()
            && self.children.is_empty()
            && self.messages.is_empty()
            && self.omitted == 0
    }
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct MemberRow {
    /// `@sessionCode-agent` for a member whose session identity is known, else the
    /// slug/npub `pubkey_ref` fallback (human operators, offline sessions).
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) state: crate::session_state::SessionState,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) since: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct PresenceRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) state: crate::session_state::SessionState,
    pub(in crate::fabric_context) status: String,
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
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct ImportantRow {
    pub(in crate::fabric_context) channel_ref: String,
    pub(in crate::fabric_context) message_id: String,
}

#[derive(Clone, PartialEq)]
pub(in crate::fabric_context) struct WarningRow {
    pub(in crate::fabric_context) text: String,
}
