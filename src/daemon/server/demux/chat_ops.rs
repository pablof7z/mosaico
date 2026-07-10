use super::*;

/// Kind:9 operations whose durable idempotency belongs to their handlers.
pub(super) fn dispatch(state: &Arc<DaemonState>, event: &Event) {
    if event.kind.as_u16() != crate::fabric::nip29::wire::KIND_CHAT {
        return;
    }
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(event) {
        tracing::info!(
            event_id = %&event.id.to_hex()[..8],
            parent = %op.parent,
            child = %op.child_h,
            "dispatching orchestration handler"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { handle_orchestration(&st, &ev, op).await });
    } else if let Some(op) = crate::fabric::nip29::session_dispatch::parse_session_dispatch(event) {
        tracing::info!(
            event_id = %&event.id.to_hex()[..8],
            route_channel = %op.route_channel,
            workspace = %op.target.workspace,
            "dispatching session dispatch handler"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { handle_session_dispatch(&st, &ev, op).await });
    } else if is_management_command_for_backend(state, event) {
        tracing::info!(
            event_id = %&event.id.to_hex()[..8],
            "dispatching management command handler"
        );
        let st = state.clone();
        let ev = event.clone();
        tokio::spawn(async move { handle_management_command(&st, &ev).await });
    }
}
