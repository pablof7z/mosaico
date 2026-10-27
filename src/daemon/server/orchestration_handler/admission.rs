//! Orchestration admission, run after the targets are claimed.

use super::super::*;
use super::{AddAgentsOp, ClaimOutcome};

/// `None` admits. Admin authority is read from the cache the retained
/// group-records observation keeps current: named by the parent's or child's
/// relay-signed kind:39001. A signer the roster does not (yet) name, a store
/// error, or an unreachable relay retries: one group record can arrive before
/// the admin list, so "not an admin" is never terminal and the attempt cap
/// bounds it. Only a child that declares a different parent is terminal.
pub(super) async fn admission(
    state: &Arc<DaemonState>,
    event: &Event,
    op: &AddAgentsOp,
) -> Option<ClaimOutcome> {
    let event_id = event.id.to_hex();
    let short = &event_id[..event_id.len().min(8)];
    let signer = event.pubkey.to_hex();
    let admin = state.with_store(|s| {
        Ok::<_, anyhow::Error>(
            s.is_channel_admin(&op.parent, &signer)? || s.is_channel_admin(&op.child_h, &signer)?,
        )
    });
    match admin {
        Ok(true) => {}
        Ok(false) => {
            tracing::debug!(
                event_id = %short,
                signer = %crate::util::pubkey_short(&signer),
                parent = %op.parent,
                child = %op.child_h,
                "orchestration signer is not (yet) an admin"
            );
            return Some(ClaimOutcome::Retry(
                "signer is not (yet) a parent or child admin",
            ));
        }
        Err(error) => {
            tracing::debug!(event_id = %short, error = %format!("{error:#}"), "orchestration admin lookup failed");
            return Some(ClaimOutcome::Retry("admin state could not be verified"));
        }
    }

    match state.provider().try_fetch_group_parent(&op.child_h).await {
        Ok(Some(declared)) if declared != op.parent && op.parent != op.child_h => {
            tracing::debug!(
                event_id = %short,
                child = %op.child_h,
                declared_parent = %declared,
                expected_parent = %op.parent,
                "orchestration child declares a different parent"
            );
            Some(ClaimOutcome::Failed("child declares a different parent"))
        }
        Ok(_) => None,
        Err(error) => {
            tracing::debug!(event_id = %short, child = %op.child_h, error = %format!("{error:#}"), "relay parent lookup failed");
            Some(ClaimOutcome::Retry(
                "relay parent state could not be verified",
            ))
        }
    }
}
