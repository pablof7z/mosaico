//! Portable PTY supervisor and client surface for reattachable agent sessions.

mod client;
mod exit_record;
mod launch;
mod meta;
mod presentation;
mod supervisor;

pub use client::{attach, attach_stream, inject, is_live, kill, list, resize, AttachStream};
pub(crate) use exit_record::{
    persist as persist_exit_report, read_all as read_exit_reports, remove as remove_exit_report,
    SupervisorExitReport,
};
pub(crate) use launch::new_endpoint_id;
pub use launch::{spawn_session, SpawnSessionArgs};
pub use meta::{
    endpoint_socket, read_all_metadata, session_dir, session_socket, write_metadata, LaunchMetadata,
};
pub(crate) use meta::{owned_supervisor_state, OwnedSupervisorState};
pub(crate) use presentation::{
    kill_if_headless_at, presentation_observation, ConditionalKillOutcome, PresentationSnapshot,
    PresentationUnavailable,
};
pub use supervisor::{run_supervisor, SupervisorArgs};

/// Explicit destructive executor used only by the daemon's termination
/// authority after it has classified the caller's intent.
pub(crate) fn terminate_explicit_owned_supervisor(endpoint_id: &str) -> anyhow::Result<bool> {
    meta::terminate_owned_supervisor(endpoint_id)
}
