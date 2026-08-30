use super::StatusCommand;
use crate::domain::{AgentRef, Status};

pub(super) fn to_status(cmd: &StatusCommand, ttl_secs: u64, now: u64, offline: bool) -> Status {
    Status {
        agent: AgentRef::new(cmd.pubkey.clone(), cmd.slug.clone()),
        channels: cmd.channels.clone(),
        host: cmd.host.clone(),
        workspace: cmd.workspace.clone(),
        branch: cmd.branch.clone(),
        title: cmd.title.clone(),
        activity: String::new(),
        state: if offline {
            crate::session_state::SessionState::Offline
        } else {
            cmd.state
        },
        state_since: if offline || cmd.state == crate::session_state::SessionState::Offline {
            now
        } else {
            cmd.state_since
        },
        rel_cwd: cmd.rel_cwd.clone(),
        // NIP-40 expiry is a lease, not a deletion command. An event that
        // expires at construction cannot enter Fava custody, and a queued
        // status may not reach the relay until later. Keep the final offline
        // status valid for the normal lease so observers see the transition
        // before it naturally ages off.
        expires_at: Some(now.saturating_add(ttl_secs)),
        dispatch_event: cmd.dispatch_event.clone(),
    }
}
