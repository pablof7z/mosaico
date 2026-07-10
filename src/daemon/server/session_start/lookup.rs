use crate::state::Store;

/// The top-level root channel for a route scope.
pub(super) fn work_root_for_scope(s: &Store, scope: &str) -> String {
    s.root_channel_of(scope)
        .ok()
        .flatten()
        .unwrap_or_else(|| scope.to_string())
}

/// The PTY endpoint currently bound to a session, via its `pty_session` alias.
pub(super) fn session_endpoint(s: &Store, session_id: &str) -> Option<String> {
    s.aliases_for_session(session_id)
        .ok()?
        .into_iter()
        .find(|a| a.external_id_kind == "pty_session")
        .map(|a| a.external_id)
}
