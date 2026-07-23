#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentWhoView {
    pub(super) self_name: String,
    pub(super) self_host: String,
    pub(super) headless: bool,
    pub(super) agents: Vec<AgentCapabilityView>,
    pub(super) workspaces: Vec<WorkspaceView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AgentCapabilityView {
    pub(super) name: String,
    pub(super) about: String,
    pub(super) workspaces: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceView {
    pub(super) name: String,
    pub(super) channel: String,
    pub(super) path: String,
    pub(super) about: String,
    pub(super) member_count: usize,
    pub(super) expanded: bool,
    pub(super) members: Vec<MemberView>,
    pub(super) channels: Vec<ChannelView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ChannelView {
    pub(super) name: String,
    pub(super) id: String,
    pub(super) about: String,
    pub(super) member_count: usize,
    pub(super) expanded: bool,
    pub(super) members: Vec<MemberView>,
    pub(super) children: Vec<ChannelView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MemberKind {
    Agent,
    Human,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MemberView {
    pub(super) kind: MemberKind,
    pub(super) name: String,
    pub(super) state: crate::session_state::SessionState,
    pub(super) status: String,
    pub(super) since: String,
}
