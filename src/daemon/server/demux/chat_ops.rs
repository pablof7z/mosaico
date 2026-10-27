use super::*;

/// Kind:9 operations whose durable idempotency belongs to their handlers.
///
/// Orchestration and session-dispatch targets are claimed here, synchronously
/// and before any relay or subscription work; only the claimed run is spawned.
/// When this returns, every due target of the event is either `processing` or
/// was not this backend's to claim, which is what lets the retry driver rely
/// on claim rows instead of inferring whether a handler ran.
pub(super) fn dispatch(state: &Arc<DaemonState>, event: &Event) {
    if event.kind.as_u16() != crate::fabric::nip29::wire::KIND_CHAT {
        return;
    }
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(event) {
        let claimed = claim_orchestration(state, event, &op);
        if claimed.is_empty() {
            return;
        }
        tracing::debug!(
            event_id = %&event.id.to_hex()[..8],
            parent = %op.parent,
            child = %op.child_h,
            targets = claimed.len(),
            "dispatching claimed orchestration targets"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { run_orchestration(&st, &ev, op, claimed).await });
    } else if let Some(op) = crate::fabric::nip29::session_dispatch::parse_session_dispatch(event) {
        let Some(claimed) = claim_session_dispatch(state, event, &op) else {
            return;
        };
        tracing::debug!(
            event_id = %&event.id.to_hex()[..8],
            route_channel = %op.route_channel,
            workspace = %op.target.workspace,
            "dispatching claimed session dispatch"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { run_session_dispatch(&st, &ev, op, claimed).await });
    } else if is_management_command_for_backend(state, event) {
        // Pre-claim match: can fire more than once for the same event, so keep it
        // at debug and don't imply execution. The authoritative single-execution
        // log is emitted post-claim inside handle_management_command (#375).
        tracing::debug!(
            event_id = %&event.id.to_hex()[..8],
            "management command matched; claiming"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { handle_management_command(&st, &ev).await });
    }
}

/// Every claim key this backend's `dispatch` of `event` would use. A stored
/// driven claim of the event outside this set can never be claimed again.
pub(super) fn driven_claim_keys(state: &Arc<DaemonState>, event: &Event) -> Vec<String> {
    let backend_pk = state.backend_pubkey();
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(event) {
        orchestration_claim_keys(&backend_pk, &op)
            .into_iter()
            .map(|(_, key)| key)
            .collect()
    } else if let Some(op) = crate::fabric::nip29::session_dispatch::parse_session_dispatch(event) {
        session_dispatch_claim_key(&backend_pk, &op)
            .into_iter()
            .collect()
    } else {
        Vec::new()
    }
}
