use crate::daemon::server::DaemonState;
use crate::state::Session;
use anyhow::Result;

pub(super) fn parent_hint(state: &DaemonState, session: &Session) -> Option<String> {
    let relay_parent =
        state.with_store(|store| store.channel_parent(&session.channel_h).ok().flatten());
    crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        Some(&session.readiness_parent),
        &session.channel_h,
    )
}

pub(super) fn workspace(state: &DaemonState, session: &Session) -> Result<String> {
    state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).root_for_session(session)
    })
}
