use super::*;

/// The canonical, replayable inputs the fabric view derives from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ViewInputs {
    pub(crate) meta: MetaInput,
    pub(crate) members: MembersInput,
    pub(crate) presence: PresenceInput,
    pub(crate) messages: MessagesInput,
    #[serde(default)]
    pub(crate) reactions: ReactionsInput,
}

impl ViewInputs {
    /// Whether the caller forced a render (suppresses the empty-snapshot gate).
    pub(crate) fn force(&self) -> bool {
        self.meta.force
    }

    pub(crate) fn turn_count(&self) -> u64 {
        self.meta
            .self_row
            .as_ref()
            .map(|row| row.turn_count)
            .unwrap_or_default()
    }
}

/// Channel/subchannel metadata + per-render identity (all now/cursor-free).
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetaInput {
    pub(in crate::fabric_context) self_row: Option<SelfCap>,
    pub(in crate::fabric_context) hosts: Vec<HostCap>,
    pub(in crate::fabric_context) workspaces: Vec<WorkspaceCap>,
    pub(in crate::fabric_context) joined_channels: BTreeSet<String>,
    pub(in crate::fabric_context) current_workspace: String,
    pub(in crate::fabric_context) warnings: Vec<String>,
    pub(in crate::fabric_context) self_pubkey: String,
    pub(in crate::fabric_context) self_ref: String,
    /// This daemon's host label for non-session fallback refs.
    #[serde(default)]
    pub(in crate::fabric_context) local_host: String,
    pub(in crate::fabric_context) force: bool,
}

/// Presence/status rows (superset, updated_at DESC) with the fields the render
/// keys on: state/activity/title plus last_seen/updated_at/expiration.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PresenceInput {
    pub(in crate::fabric_context) statuses: BTreeMap<String, Vec<StatusCap>>,
}

/// Chat/mentions: per-channel captured events + forced (inbox) seeds.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MessagesInput {
    pub(in crate::fabric_context) channels: BTreeMap<String, MsgBundle>,
}

/// Reactions on the caller's own recent messages (a cursor-independent superset;
/// the cursor delta is applied at assemble time).
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReactionsInput {
    pub(in crate::fabric_context) rows: Vec<super::super::reactions::ReactionCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SelfCap {
    pub(in crate::fabric_context) name: String,
    #[serde(default)]
    pub(in crate::fabric_context) host: String,
    #[serde(default)]
    pub(in crate::fabric_context) headless: bool,
    #[serde(default)]
    pub(in crate::fabric_context) title: String,
    #[serde(default)]
    pub(in crate::fabric_context) workspace: String,
    #[serde(default)]
    pub(in crate::fabric_context) branch: String,
    #[serde(default)]
    pub(in crate::fabric_context) turn_count: u64,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SummaryCap {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) about: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentCap {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HostCap {
    pub(in crate::fabric_context) name: String,
    #[serde(default)]
    pub(in crate::fabric_context) roots: Vec<String>,
    pub(in crate::fabric_context) agents: Vec<AgentCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChannelCap {
    pub(in crate::fabric_context) h: String,
    #[serde(default)]
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) updated_at: u64,
    pub(in crate::fabric_context) latest_message_at: Option<u64>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MsgBundle {
    pub(in crate::fabric_context) events: Vec<EvCap>,
    pub(in crate::fabric_context) forced: Vec<EvCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvCap {
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) channel_ref: String,
    pub(in crate::fabric_context) from_ref: String,
    pub(in crate::fabric_context) recipient_refs: Vec<String>,
    pub(in crate::fabric_context) created_at: u64,
    pub(in crate::fabric_context) body: String,
    #[serde(default)]
    pub(in crate::fabric_context) attachment_dir: String,
    /// Self-mention derived from the event's OWN `p` tags (always false for a
    /// forced seed, whose mention intent is carried by `forced_mention`).
    pub(in crate::fabric_context) mentions_self: bool,
    /// A forced (inbox) seed that was flagged as a direct mention.
    pub(in crate::fabric_context) forced_mention: bool,
    pub(in crate::fabric_context) needs_reply_nudge: bool,
}
