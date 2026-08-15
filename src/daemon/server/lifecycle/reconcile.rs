use super::*;

/// Reconcile leftover state a prior daemon left open before the store is shared
/// with the engine: native turn attempts and extension-owned delivery leases.
/// Each branch warns when it recovered anything, so startup logs explain any
/// work the prior process did not finish.
pub(super) fn leftover_startup_state(store: &Store) -> Result<()> {
    let reconciled_attempts = store.reconcile_open_native_turn_attempts(now_secs())?;
    if reconciled_attempts > 0 {
        tracing::warn!(
            reconciled_attempts,
            "reconciled native turn attempts left open by the prior daemon"
        );
    }
    let requeued_extension_leases = store.reenqueue_extension_leases(None)?.len();
    if requeued_extension_leases > 0 {
        tracing::warn!(
            requeued_extension_leases,
            "requeued extension deliveries left leased by the prior daemon"
        );
    }
    Ok(())
}
