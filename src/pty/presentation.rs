//! Typed presentation-state protocol for a live PTY supervisor.

use super::meta;
use serde::de::DeserializeOwned;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationSnapshot {
    pub(crate) attached_clients: u64,
    pub(crate) attachment_epoch: u64,
    pub(crate) changed_at: u64,
}

impl PresentationSnapshot {
    pub(crate) fn is_headless(self) -> bool {
        self.attached_clients == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ConditionalKillOutcome {
    Killed { presentation: PresentationSnapshot },
    PresentationChanged { presentation: PresentationSnapshot },
}

#[derive(Debug)]
pub(crate) struct PresentationUnavailable {
    detail: String,
}

impl PresentationUnavailable {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for PresentationUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PTY presentation unavailable: {}", self.detail)
    }
}

impl std::error::Error for PresentationUnavailable {}

/// Return the supervisor's current presentation state. Failure is distinct from
/// a reachable supervisor reporting zero attached clients.
pub(crate) fn presentation_snapshot(
    id_or_path: &str,
) -> Result<PresentationSnapshot, PresentationUnavailable> {
    request(id_or_path, "PRESENTATION\n", Duration::from_millis(100))
}

pub(crate) fn presentation_observation(
    id_or_path: &str,
) -> Result<PresentationSnapshot, PresentationUnavailable> {
    presentation_snapshot(id_or_path)
}

/// Kill only if the supervisor still has no attached clients at `expected_epoch`.
/// The supervisor evaluates the predicate and confirms child exit while holding
/// the same attachment-state lock used by attach and detach transitions.
pub(crate) fn kill_if_headless_at(
    id_or_path: &str,
    expected_epoch: u64,
) -> Result<ConditionalKillOutcome, PresentationUnavailable> {
    request(
        id_or_path,
        &format!("KILL_IF_HEADLESS {expected_epoch}\n"),
        Duration::from_secs(4),
    )
}

fn request<T: DeserializeOwned>(
    id_or_path: &str,
    command: &str,
    timeout: Duration,
) -> Result<T, PresentationUnavailable> {
    let path = meta::resolve_socket(id_or_path);
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| PresentationUnavailable::new(format!("{}: {error}", path.display())))?;
    stream
        .write_all(command.as_bytes())
        .and_then(|_| stream.flush())
        .map_err(|error| PresentationUnavailable::new(format!("request write failed: {error}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| PresentationUnavailable::new(format!("read timeout failed: {error}")))?;
    let mut response = String::new();
    let read = BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| PresentationUnavailable::new(format!("response read failed: {error}")))?;
    if read == 0 {
        return Err(PresentationUnavailable::new(
            "supervisor closed without a response",
        ));
    }
    serde_json::from_str(response.trim()).map_err(|error| {
        PresentationUnavailable::new(format!("invalid supervisor response: {error}"))
    })
}

#[cfg(test)]
#[path = "presentation/tests.rs"]
mod tests;
