//! Portable PTY supervisor and client surface for reattachable agent sessions.

mod client;
mod launch;
mod meta;
mod supervisor;

pub use client::{attach, attach_stream, inject, is_live, kill, list, resize, AttachStream};
pub(crate) use launch::new_session_id;
pub use launch::{spawn_session, SpawnSessionArgs};
pub use meta::{
    endpoint_socket, read_all_metadata, session_dir, session_socket, write_metadata, LaunchMetadata,
};
pub use supervisor::{run_supervisor, SupervisorArgs};
