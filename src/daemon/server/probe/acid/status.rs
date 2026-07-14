use std::sync::Arc;

use anyhow::{Context, Result};

use super::DaemonState;
use crate::reconcile::{InputFact, StatusDrive};

pub(super) fn current_arm_at(state: &Arc<DaemonState>, session_id: &str) -> Result<u64> {
    state
        .status
        .lock()
        .expect("status mutex poisoned")
        .current_arm_at(session_id)
        .with_context(|| format!("probe acid: no status arm for `{session_id}`"))
}

pub(super) fn remove_tick_arm(
    state: &Arc<DaemonState>,
    pubkey: String,
    automatic_delivery: bool,
    cause: &str,
) -> Result<InputFact> {
    if !cause.ends_with("/arm") {
        anyhow::bail!("probe acid: unsupported status cause `{cause}`");
    }
    Ok(InputFact::StatusDrive(StatusDrive::Tick {
        at: current_arm_at(state, &pubkey)?,
        pubkey,
        automatic_delivery,
    }))
}
