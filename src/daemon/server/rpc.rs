//! RPC handler families extracted from `server.rs` (issue #12, EPIC-server-001).
//!
//! Each submodule owns one cohesive handler family. Handlers are `pub(super)`
//! inside their submodule and re-exported here as `pub` so the dispatch
//! table in `server.rs` can call them as `rpc::rpc_*`.

pub(super) mod agents;
pub(super) mod channel_members;

pub(super) use agents::{rpc_agents_list_sessions, rpc_agents_roster};
pub use channel_members::{
    rpc_channel_add_member, rpc_channel_members, rpc_channel_remove_member, rpc_root_channels,
};
