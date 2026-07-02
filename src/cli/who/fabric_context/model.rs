#[derive(Default)]
pub(super) struct FabricView {
    pub(super) self_row: Option<SelfRow>,
    pub(super) project: ProjectRow,
    pub(super) agents: Vec<AgentRow>,
    pub(super) channels: Vec<ChannelBlock>,
    pub(super) inactive: Vec<InactiveChannelRow>,
    pub(super) important: Vec<ImportantRow>,
    pub(super) warnings: Vec<WarningRow>,
}

#[derive(Default)]
pub(super) struct ProjectRow {
    pub(super) name: String,
    pub(super) about: String,
}

pub(super) struct SelfRow {
    pub(super) agent: String,
    pub(super) backend: String,
    pub(super) session_id: String,
}

pub(super) struct AgentRow {
    pub(super) reference: String,
    pub(super) about: String,
}

pub(super) struct ChannelBlock {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) about: String,
    pub(super) active: bool,
    pub(super) members: Vec<MemberRow>,
    pub(super) presence: Vec<PresenceRow>,
    pub(super) subchannels: Vec<ChannelSummaryRow>,
    pub(super) messages: Vec<MessageRow>,
    pub(super) omitted: usize,
}

pub(super) struct MemberRow {
    pub(super) reference: String,
    pub(super) status: String,
    pub(super) seen: String,
}

pub(super) struct PresenceRow {
    pub(super) reference: String,
    pub(super) status: String,
    pub(super) seen: String,
}

pub(super) struct ChannelSummaryRow {
    pub(super) name: String,
    pub(super) about: String,
}

pub(super) struct MessageRow {
    pub(super) id: String,
    pub(super) channel: String,
    pub(super) from: String,
    pub(super) age: String,
    pub(super) body: String,
    pub(super) mention: bool,
    pub(super) truncated: bool,
}

pub(super) struct InactiveChannelRow {
    pub(super) name: String,
    pub(super) about: String,
    pub(super) last_active: String,
}

pub(super) struct ImportantRow {
    pub(super) channel: String,
    pub(super) message_id: String,
}

pub(super) struct WarningRow {
    pub(super) text: String,
}
