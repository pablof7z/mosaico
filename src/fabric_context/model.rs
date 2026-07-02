#[derive(Default)]
pub(in crate::fabric_context) struct FabricView {
    pub(in crate::fabric_context) self_row: Option<SelfRow>,
    pub(in crate::fabric_context) project: ProjectRow,
    pub(in crate::fabric_context) agents: Vec<AgentRow>,
    pub(in crate::fabric_context) channels: Vec<ChannelBlock>,
    pub(in crate::fabric_context) inactive: Vec<InactiveChannelRow>,
    pub(in crate::fabric_context) important: Vec<ImportantRow>,
    pub(in crate::fabric_context) warnings: Vec<WarningRow>,
}

#[derive(Default)]
pub(in crate::fabric_context) struct ProjectRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
}

pub(in crate::fabric_context) struct SelfRow {
    pub(in crate::fabric_context) agent: String,
    pub(in crate::fabric_context) backend: String,
    pub(in crate::fabric_context) session_id: String,
}

pub(in crate::fabric_context) struct AgentRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) about: String,
}

pub(in crate::fabric_context) struct ChannelBlock {
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) active: bool,
    pub(in crate::fabric_context) members: Vec<MemberRow>,
    pub(in crate::fabric_context) presence: Vec<PresenceRow>,
    pub(in crate::fabric_context) subchannels: Vec<ChannelSummaryRow>,
    pub(in crate::fabric_context) messages: Vec<MessageRow>,
    pub(in crate::fabric_context) omitted: usize,
}

pub(in crate::fabric_context) struct MemberRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) seen: String,
}

pub(in crate::fabric_context) struct PresenceRow {
    pub(in crate::fabric_context) reference: String,
    pub(in crate::fabric_context) status: String,
    pub(in crate::fabric_context) seen: String,
}

pub(in crate::fabric_context) struct ChannelSummaryRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
}

pub(in crate::fabric_context) struct MessageRow {
    pub(in crate::fabric_context) id: String,
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) from: String,
    pub(in crate::fabric_context) age: String,
    pub(in crate::fabric_context) body: String,
    pub(in crate::fabric_context) mention: bool,
    pub(in crate::fabric_context) truncated: bool,
}

pub(in crate::fabric_context) struct InactiveChannelRow {
    pub(in crate::fabric_context) name: String,
    pub(in crate::fabric_context) about: String,
    pub(in crate::fabric_context) last_active: String,
}

pub(in crate::fabric_context) struct ImportantRow {
    pub(in crate::fabric_context) channel: String,
    pub(in crate::fabric_context) message_id: String,
}

pub(in crate::fabric_context) struct WarningRow {
    pub(in crate::fabric_context) text: String,
}
