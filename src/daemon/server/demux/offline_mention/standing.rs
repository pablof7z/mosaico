use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    Confirmed,
    Repair,
    Pending,
    Declined,
}

fn gate(store: &crate::state::Store, pubkey: &str, channel: &str) -> Result<Gate> {
    let session = store
        .get_session(pubkey)?
        .with_context(|| format!("exact recovery target {pubkey} disappeared"))?;
    if !session.is_running() {
        anyhow::bail!("exact recovery target {pubkey} stopped before relay admission");
    }
    let routed = store.has_session_route(pubkey, channel)?;
    let standing = store.get_session_standing(pubkey, channel)?;
    if !routed {
        return Ok(
            if standing.is_some_and(|row| row.state == crate::state::StandingState::Absent) {
                Gate::Declined
            } else {
                Gate::Pending
            },
        );
    }
    Ok(
        if standing.is_some_and(|row| row.state == crate::state::StandingState::Member) {
            Gate::Confirmed
        } else {
            Gate::Repair
        },
    )
}

pub(super) async fn confirm(state: &Arc<DaemonState>, pubkey: &str, channel: &str) -> Result<()> {
    let _lane = state.standing_sync.lock().await;
    match state.with_store(|store| gate(store, pubkey, channel))? {
        Gate::Confirmed | Gate::Declined => return Ok(()),
        Gate::Pending => {
            anyhow::bail!("exact recovery target {pubkey} has no committed route to {channel}")
        }
        Gate::Repair => {}
    }
    let session = state
        .with_store(|store| store.get_session(pubkey))?
        .with_context(|| format!("exact recovery target {pubkey} disappeared"))?;
    let outcome = state.provider.grant_member_confirmed(channel, pubkey).await;
    if !outcome.is_confirmed() {
        anyhow::bail!("relay admission was not confirmed: {outcome:?}");
    }
    if !super::super::super::managed_lifecycle::commit_confirmed_admission(
        state,
        pubkey,
        channel,
        session.runtime_generation,
        session.lifecycle_epoch,
    )
    .await?
    {
        anyhow::bail!("session changed during relay admission; cleanup was scheduled");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RegisterSession;

    fn running(store: &crate::state::Store, channel: &str) {
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: "pk".into(),
                observed_harness: "codex".into(),
                agent_slug: "agent".into(),
                launch_channel_h: channel.into(),
                work_root: channel.into(),
                child_pid: None,
                now: 1,
            })
            .unwrap();
    }

    #[test]
    fn explicit_leave_declines_late_recovery_repair() {
        let store = crate::state::Store::open_memory().unwrap();
        running(&store, "room");
        store.revoke_route_and_mark_absent("pk", "room", 2).unwrap();

        assert_eq!(gate(&store, "pk", "room").unwrap(), Gate::Declined);
    }

    #[test]
    fn pending_spawn_without_a_route_retries_instead_of_minting_membership() {
        let store = crate::state::Store::open_memory().unwrap();
        running(&store, "");

        assert_eq!(gate(&store, "pk", "room").unwrap(), Gate::Pending);
    }
}
