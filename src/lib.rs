//! tenex-edge — citizenship for your agents.
//!
//! A host-neutral substrate: durable agent identity + awareness + messaging on
//! the Nostr fabric. Nothing here knows about any host (no `pc`, no `claude`);
//! hosts integrate from the outside via the CLI, hooks, and a skill.
//!
//! Layering (M1 §2), each knowing only what is below it:
//!   cli -> runtime -> { domain, codec, transport, state, distill }
//!   config / identity / project are leaf utilities.

pub mod config;
pub mod domain;
pub mod identity;
pub mod idref;
pub mod injection;
pub mod llmconfig;
pub mod project;
pub mod session;
pub mod util;

pub mod cli;
pub mod codec;
pub mod daemon;
pub mod distill;
pub mod fabric;
pub mod runtime;
pub mod state;
pub mod tmux;
pub mod transcript;
pub mod transport;
