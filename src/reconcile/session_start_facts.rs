use serde::{Deserialize, Serialize};

/// Canonical advisory input for the daemon's session-start path.
///
/// Host-only work that Trellis cannot prove, such as resolving channels,
/// reserving signers, and detecting a running task, enters as observed fields.
/// The session-start graph derives staged intents from those observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStartRequestFact {
    pub session_id: String,
    pub agent: String,
    pub harness: String,
    pub external_id_kind: String,
    pub external_id: String,
    pub native_id: String,
    pub work_root: String,
    pub channel_h: String,
    pub channel_for_upsert: String,
    pub rel_cwd: String,
    pub room_parent: Option<String>,
    pub watch_pid: Option<i32>,
    pub tmux_pane: Option<String>,
    pub ring_doorbell: bool,
    pub base_pubkey: String,
    pub signer_pubkey: String,
    pub signer_label: String,
    pub signer_ordinal: u32,
    pub already_running: bool,
    pub channel_already_subscribed: bool,
    pub at: u64,
}

/// Outcome fact for a request that reached the advisory plan but failed while
/// the host executed one of the imperative effects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStartFailedFact {
    pub session_id: String,
    pub stage: String,
    pub error: String,
    pub at: u64,
}
