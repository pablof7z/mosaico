use super::*;

/// A PID correlates host runtimes but never chooses public identity. When a new
/// native session starts on a watched PID, retire the prior pubkey generation
/// before the PID locator is rebound to the new owner.
pub(super) fn retire_conflicting_pid_runtime(
    state: &Arc<DaemonState>,
    new_pubkey: &str,
    agent_slug: &str,
    harness: &str,
    watch_pid: Option<i32>,
    new_work_root: &str,
) -> Result<()> {
    let Some(pid) = watch_pid.map(|pid| pid.to_string()) else {
        return Ok(());
    };
    let Some(old_pubkey) = state.with_store(|store| {
        store.resolve_pubkey_by_locator(harness, crate::state::LOCATOR_PID, &pid)
    })?
    else {
        return Ok(());
    };
    if old_pubkey == new_pubkey {
        return Ok(());
    }
    let Some(old) = state.with_store(|store| store.get_session(&old_pubkey))? else {
        return Ok(());
    };
    if !old.alive || old.agent_slug != agent_slug {
        return Ok(());
    }
    let old_work_root = state.with_store(|store| {
        crate::daemon::server::resolution::work_root_for(store, &old.channel_h)
    });
    if old_work_root != new_work_root {
        return Ok(());
    }

    tracing::info!(
        old_pubkey,
        new_pubkey,
        runtime_generation = old.runtime_generation,
        pid,
        "retiring prior runtime generation on reused host pid"
    );
    cancel_session(state, &old_pubkey);
    state.with_store(|store| {
        store.touch_session(&old_pubkey, now_secs())?;
        store.mark_dead_if_generation(&old_pubkey, old.runtime_generation)
    })?;
    Ok(())
}
