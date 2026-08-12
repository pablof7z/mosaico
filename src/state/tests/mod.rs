//! Persistence-foundation tests: canonical session identity and host-local
//! channel, delivery, and runtime facts. NMP owns relay event semantics.
//!
//! Split by theme into sibling files to stay under the repo's per-file LOC
//! ceiling; shared fixtures live here.

use super::*;

fn reg(harness: &str, ext: &str, channel: &str) -> RegisterSession {
    RegisterSession {
        pubkey: ext.into(),
        observed_harness: harness.into(),
        agent_slug: "agent".into(),
        launch_channel_h: channel.into(),
        work_root: channel.into(),
        child_pid: Some(42),
        now: 1000,
    }
}

mod agent_usage;
mod channels_tree;
mod identity_projection_and_roots;
mod inbox_ledger;
mod retention;
mod runtime_admission;
mod session_coaching;
mod session_identity;
