//! mosaico — citizenship for your agents.
//!
//! A host-neutral substrate: durable agent identity + awareness + messaging on
//! the Nostr fabric. Nothing here knows about any host (no `pc`, no `claude`);
//! hosts integrate from the outside via the CLI, hooks, and a skill.
//!
//! Layering, each knowing only what is below it:
//!   cli -> runtime -> { domain, codec, NMP, state }
//!   config / identity / channel are leaf utilities.

mod agent_about;
pub mod agent_catalog;
pub(crate) mod agent_inventory;
mod attachment;
mod channel_about;
mod channel_name;
mod channel_nudge;
mod channel_ref;
pub mod command_forensics;
pub mod config;
pub(crate) mod console_style;
pub(crate) mod delivery_seam;
pub mod domain;
pub mod explain;
mod fabric_context;
mod goose_integration;
pub mod harness;
pub(crate) mod host_env;
pub mod identity;
pub mod idref;
pub mod injection;
pub mod instrument;
pub(crate) mod liveness;
pub mod logging;
mod nmp_host;
pub mod profile;
mod secret_scrub;
pub mod session;
pub(crate) mod session_presence;
pub mod session_state;
pub mod slug;
pub mod util;
pub(crate) mod workspace;

pub mod cli;
pub mod daemon;
pub mod fabric;
mod presence_publisher;
pub mod pty;
pub mod reconcile;
pub mod relay_log;
pub mod rpc_harness;
pub mod runtime;
pub mod session_host;
pub(crate) mod session_title;
pub mod state;

mod expired_sessions;
mod turn_context;
mod who_aggregation;
mod who_snapshot;

#[cfg(test)]
pub(crate) mod test_env;
