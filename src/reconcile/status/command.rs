use super::StatusEffect;
use crate::session_state::SessionState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusCommand {
    pub pubkey: String,
    pub channels: Vec<String>,
    pub title: String,
    pub state: SessionState,
    pub state_since: u64,
    pub host: String,
    pub slug: String,
    pub rel_cwd: String,
    pub dispatch_event: Option<String>,
}

pub struct StatusOutcome {
    pub effects: Vec<StatusEffect>,
    pub revision: u64,
    pub pubkey: Option<String>,
}
