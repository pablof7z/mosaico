//! tenex-edge — citizenship for your agents.
//!
//! A host-neutral substrate: durable agent identity + awareness + messaging on
//! the Nostr fabric. Nothing here knows about any host (no `pc`, no `claude`);
//! hosts integrate from the outside via the CLI, hooks, and a skill.
//!
//! Layering (M1 §2), each knowing only what is below it:
//!   cli -> runtime -> { domain, codec, transport, state, distill }
//!   config / identity / project are leaf utilities.

mod applog;
mod channel_about;
pub mod command_forensics;
pub mod config;
pub mod domain;
pub mod explain;
mod fabric_context;
pub mod identity;
pub mod idref;
pub mod injection;
pub mod instrument;
pub mod llmconfig;
pub mod logging;
pub(crate) mod outbox_seam;
pub mod profile;
pub mod project;
pub mod session;
pub mod util;

pub mod cli;
pub mod daemon;
pub mod distill;
pub mod fabric;
pub mod relay_log;
pub mod replay_capsules;
pub mod runtime;
pub mod state;
pub mod status_seam;
pub mod tmux;
pub mod transcript;
pub mod transport;

pub mod reconcile;

mod turn_context;
mod who_snapshot;

#[cfg(test)]
pub(crate) mod test_env;
