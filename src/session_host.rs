//! Reattachable session hosting for mosaico.
//!
//! Two public surfaces:
//!
//!   • `ring_doorbells(state)` — called after chat delivery events.
//!     Finds sessions that have unread chat mentions + a live PTY endpoint, then
//!     injects the rendered pending messages into the agent.
//!
//!   • `spawn_agent(state, slug, root, launch_args)` — spawns a new
//!     PTY-hosted harness. Manual spawns start clean; no prompt is injected.
//!
//! Delivery is fail-open: endpoint failures are logged and pending inbox rows are
//! returned to the queue so another path can deliver them.

mod admission;
mod agent_env;
mod delivery;
mod exec;
mod launch;
mod registry;
pub mod transport;

pub use delivery::{
    deliver_spawn_prompt, inject_pending_messages_pty, inject_spawn_message, ring_doorbells,
};
pub(crate) use delivery::{session_has_live_delivery_path, session_is_headless};
pub(crate) use exec::{
    agent_supports_headless_exec, bind_native_id_from_log, spawn_agent_exec, ExecLaunch,
};
pub use launch::{
    resume_agent, resume_agent_in_channel, spawn_dispatched_ephemeral_agent, spawn_ephemeral_agent,
    DispatchedSpawn,
};
pub(crate) use launch::{spawn_agent, SpawnRequest};
pub use registry::spawnable_agents;

#[cfg(test)]
#[path = "session_host/tests.rs"]
mod resume_command_tests;
