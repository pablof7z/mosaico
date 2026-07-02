//! TMUX control plane for tenex-edge.
//!
//! Two public surfaces:
//!
//!   • `ring_doorbells(state)` — called after chat delivery events.
//!     Finds sessions that have unread chat mentions + a live tmux endpoint, then
//!     injects the rendered pending messages into the pane.
//!
//!   • `spawn_agent(state, slug, project, launch_args)` — spawns a new tmux window
//!     running the appropriate harness command. Manual spawns start clean — no
//!     prompt is injected.
//!
//! Fail-open everywhere: if the `tmux` binary is absent, TMUX_PANE was never set,
//! or any sub-command errors, we log to stderr (debug only) and return Ok(()).

mod delivery;
mod launch;
mod pane;
mod registry;

pub use delivery::{inject_pending_messages_pub, inject_spawn_message, ring_doorbells};
pub use launch::{resume_agent, resume_agent_in_channel, spawn_agent};
pub use pane::{list_endpoint_statuses, pane_alive_pub, set_pane_session_id, EndpointStatus};
pub use registry::spawnable_agents;

#[cfg(test)]
#[path = "tmux/tests.rs"]
mod resume_command_tests;
