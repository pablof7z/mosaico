use super::StatusCommand;
use crate::domain::{AgentRef, Status};

pub(super) fn to_status(cmd: &StatusCommand, ttl_secs: u64, now: u64, expiring: bool) -> Status {
    Status {
        agent: AgentRef::new(cmd.pubkey.clone(), cmd.slug.clone()),
        channels: cmd.channels.clone(),
        host: cmd.host.clone(),
        title: cmd.title.clone(),
        activity: String::new(),
        state: if expiring {
            crate::session_state::SessionState::Offline
        } else {
            cmd.state
        },
        state_since: if expiring || cmd.state == crate::session_state::SessionState::Offline {
            now
        } else {
            cmd.state_since
        },
        rel_cwd: cmd.rel_cwd.clone(),
        expires_at: Some(
            if expiring || cmd.state == crate::session_state::SessionState::Offline {
                now
            } else {
                now.saturating_add(ttl_secs)
            },
        ),
        dispatch_event: cmd.dispatch_event.clone(),
    }
}
