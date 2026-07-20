//! Reattachable session hosting for mosaico.
//!
//! Two public surfaces:
//!
//!   • `ring_doorbells(state)` — called after chat delivery events.
//!     Finds sessions that have unread chat mentions + a live hosted endpoint,
//!     then delivers the rendered pending messages through its transport.
//!
//!   • `spawn_agent(state, slug, root, launch_args)` — spawns a new
//!     hosted harness. Manual spawns start clean; no prompt is injected.
//!
//! Delivery is fail-open: endpoint failures are logged and pending inbox rows are
//! returned to the queue so another path can deliver them.

mod admission;
mod agent_env;
mod delivery;
mod launch;
mod native_discovery;
mod registry;
pub mod transport;

pub use delivery::{deliver_spawn_prompt, inject_pending_messages_pty, ring_doorbells};
pub(crate) use delivery::{session_has_live_delivery_path, session_is_headless};
pub(crate) use launch::spawn_ephemeral_agent_for_pubkey;
pub(crate) use launch::{
    adopt_native_session, resume_agent, resume_agent_in_channel, resume_session_record,
};
pub(crate) use launch::{spawn_agent, LaunchIntent, SpawnRequest};
pub use launch::{spawn_dispatched_ephemeral_agent, spawn_ephemeral_agent, DispatchedSpawn};
pub(crate) use native_discovery::discover as discover_native_session;
pub use registry::spawnable_agents;

#[cfg(test)]
#[path = "session_host/tests.rs"]
mod session_host_tests;
