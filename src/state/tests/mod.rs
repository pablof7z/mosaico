//! Persistence-foundation tests: canonical session identity, NIP-01 replacement,
//! NIP-40 status liveness, and unique-pubkey-per-channel membership.
//!
//! Split by theme into sibling files to stay under the repo's per-file LOC
//! ceiling; shared fixtures live here.

use super::*;

fn reg(harness: &str, ext: &str, channel: &str) -> RegisterSession {
    RegisterSession {
        pubkey: ext.into(),
        harness: harness.into(),
        agent_slug: "agent".into(),
        channel_h: channel.into(),
        child_pid: Some(42),
        transcript_path: Some("/t/x.jsonl".into()),
        now: 1000,
    }
}

mod agent_usage;
mod channels_tree;
mod identity_projection_and_roots;
mod inbox_ledger;
mod nip01_events;
mod retention;
mod session_identity;
mod status_membership;
