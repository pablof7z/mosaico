use super::*;
use crate::state::ConfirmedAdmissionCommit;

fn route_has_current_member_standing(
    state: &Arc<DaemonState>,
    session: &Session,
    channel: &str,
) -> Result<bool> {
    state.with_store(|store| {
        Ok(store.has_session_route(&session.pubkey, channel)?
            && store.has_channel_membership_snapshot(channel)?
            && store.is_channel_member(channel, &session.pubkey)?
            && store
                .get_session_standing(&session.pubkey, channel)?
                .is_some_and(|standing| {
                    standing.state == crate::state::StandingState::Member
                        && standing.session_lifecycle_epoch == session.lifecycle_epoch
                }))
    })
}

/// Join relay readiness already authorized by this exact session route.
///
/// Session start records the route before its relay confirmation completes so
/// the new process can resolve its channel immediately. A send on that exact
/// route waits for the same serialized admission instead of racing the publish
/// gate or implicitly joining an unrelated destination.
pub(in crate::daemon::server) async fn ensure_session_route_ready(
    state: &Arc<DaemonState>,
    expected: &Session,
    channel: &str,
) -> Result<()> {
    if !state.with_store(|store| store.has_session_route(&expected.pubkey, channel))? {
        return Ok(());
    }
    if route_has_current_member_standing(state, expected, channel)? {
        return Ok(());
    }

    let _lane = state.standing_sync.lock().await;
    let session = state
        .with_store(|store| store.get_session(&expected.pubkey))?
        .context("session disappeared while awaiting channel admission")?;
    anyhow::ensure!(
        session.runtime_generation == expected.runtime_generation
            && session.lifecycle_epoch == expected.lifecycle_epoch
            && session.is_running(),
        "session changed while awaiting channel admission"
    );
    if route_has_current_member_standing(state, &session, channel)? {
        return Ok(());
    }
    anyhow::ensure!(
        admission_is_current(
            state,
            &session.pubkey,
            channel,
            session.runtime_generation,
            session.lifecycle_epoch,
            false,
        ),
        "session channel admission is no longer current"
    );

    let relay_parent = state.with_store(|store| store.channel_parent(channel))?;
    let parent = crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        (!session.readiness_parent.is_empty()).then_some(session.readiness_parent.as_str()),
        channel,
    );
    let gate = tokio::time::timeout(
        Duration::from_secs(45),
        state
            .provider()
            .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
                channel,
                expect_member: &session.pubkey,
                parent_hint: parent.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            }),
    )
    .await
    .context("session channel admission timed out before send")?;
    gate.require_ready("session channel admission was not relay-confirmed before send")?;
    anyhow::ensure!(
        commit_confirmed_admission(
            state,
            &session.pubkey,
            channel,
            session.runtime_generation,
            session.lifecycle_epoch,
        )
        .await?,
        "session channel admission became stale before send"
    );
    Ok(())
}

/// Revalidate a relay-admission task while `standing_sync` is held. Existing
/// durable routes are always authoritative. A fresh launch may establish a new
/// route only if this lifecycle has not already recorded an explicit absence.
pub(in crate::daemon::server) fn admission_is_current(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    runtime_generation: u64,
    lifecycle_epoch: u64,
    allow_new_route: bool,
) -> bool {
    state
        .with_store(|store| -> Result<bool> {
            let Some(session) = store.get_session(pubkey)? else {
                return Ok(false);
            };
            if !session.is_running()
                || session.runtime_generation != runtime_generation
                || session.lifecycle_epoch != lifecycle_epoch
            {
                return Ok(false);
            }
            if store.has_session_route(pubkey, channel)? {
                return Ok(true);
            }
            if !allow_new_route {
                return Ok(false);
            }
            Ok(store
                .get_session_standing(pubkey, channel)?
                .is_none_or(|standing| standing.session_lifecycle_epoch != lifecycle_epoch))
        })
        .unwrap_or_else(|error| {
            tracing::error!(
                pubkey = %pubkey_short(pubkey),
                %channel,
                %error,
                "admission authorization revalidation failed"
            );
            false
        })
}

/// Finalize relay-confirmed membership while the caller holds `standing_sync`.
/// The exact lifecycle may already have stopped; runtime stop is not leave.
/// A stale or failed primary commit first becomes durable cleanup work, so an
/// unconfirmed compensation is retried by the standing coordinator.
pub(in crate::daemon::server) async fn commit_confirmed_admission(
    state: &Arc<DaemonState>,
    pubkey: &str,
    channel: &str,
    runtime_generation: u64,
    lifecycle_epoch: u64,
) -> Result<bool> {
    let now = now_secs();
    let primary = state.with_store(|store| {
        store.commit_confirmed_session_admission(
            pubkey,
            channel,
            runtime_generation,
            lifecycle_epoch,
            now,
        )
    });
    match primary {
        Ok(ConfirmedAdmissionCommit::Committed) => {
            reconcile_admission(state, pubkey, runtime_generation).await;
            Ok(true)
        }
        Ok(ConfirmedAdmissionCommit::Superseded) => {
            tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, lifecycle_epoch, "stale admission was superseded by newer member standing");
            Ok(false)
        }
        Ok(ConfirmedAdmissionCommit::CleanupDue(due)) => {
            compensate_due_admission(state, &due).await;
            Ok(false)
        }
        Err(primary_error) => {
            let fallback = state.with_store(|store| {
                store.schedule_confirmed_admission_cleanup(
                    pubkey,
                    channel,
                    runtime_generation,
                    lifecycle_epoch,
                    now_secs(),
                )
            });
            match fallback {
                Ok(ConfirmedAdmissionCommit::Committed) => {
                    tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, %primary_error, "admission commit reported an error but its exact durable state is present");
                    reconcile_admission(state, pubkey, runtime_generation).await;
                    Ok(true)
                }
                Ok(ConfirmedAdmissionCommit::Superseded) => {
                    tracing::warn!(pubkey = %pubkey_short(pubkey), %channel, %primary_error, "failed admission commit was superseded by newer member standing");
                    Ok(false)
                }
                Ok(ConfirmedAdmissionCommit::CleanupDue(due)) => {
                    compensate_due_admission(state, &due).await;
                    Err(primary_error).context("confirmed admission could not be committed")
                }
                Err(cleanup_error) => Err(anyhow::anyhow!(
                    "confirmed admission commit failed ({primary_error:#}); durable cleanup persistence also failed ({cleanup_error:#})"
                )),
            }
        }
    }
}

async fn reconcile_admission(state: &Arc<DaemonState>, pubkey: &str, generation: u64) {
    super::super::presence::reassert_generation(state, pubkey, generation, "channel_admitted")
        .await;
}

async fn compensate_due_admission(state: &Arc<DaemonState>, due: &crate::state::SessionStanding) {
    let removal = state
        .provider()
        .remove_member_confirmed(&due.channel_h, &due.pubkey)
        .await;
    if !removal.is_confirmed() {
        tracing::warn!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            ?removal,
            "admission compensation remains durably due"
        );
        return;
    }
    match state.with_store(|store| {
        store.mark_member_standing_absent_if_epoch(
            &due.pubkey,
            &due.channel_h,
            due.standing_epoch,
            due.session_lifecycle_epoch,
            now_secs(),
        )
    }) {
        Ok(true) => tracing::info!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            "stale confirmed admission was removed"
        ),
        Ok(false) => tracing::debug!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            "admission compensation was superseded while removal completed"
        ),
        Err(error) => tracing::error!(
            pubkey = %pubkey_short(&due.pubkey),
            channel = %due.channel_h,
            %error,
            "confirmed admission removal could not be persisted; cleanup remains due"
        ),
    }
}

pub(super) async fn reconcile_running(state: &Arc<DaemonState>) {
    let cleanup_due = match state.with_store(|store| store.list_cleanup_due_member_standing()) {
        Ok(due) => due,
        Err(error) => {
            tracing::error!(%error, "standing cleanup scan failed");
            Vec::new()
        }
    };
    for due in cleanup_due {
        let _lane = state.standing_sync.lock().await;
        let still_due = match state.with_store(|store| {
            let standing = store.get_session_standing(&due.pubkey, &due.channel_h)?;
            let routed = store.has_session_route(&due.pubkey, &due.channel_h)?;
            Ok::<_, anyhow::Error>(standing.as_ref() == Some(&due) && !routed)
        }) {
            Ok(still_due) => still_due,
            Err(error) => {
                tracing::error!(
                    pubkey = %pubkey_short(&due.pubkey),
                    channel = %due.channel_h,
                    %error,
                    "standing cleanup revalidation failed"
                );
                false
            }
        };
        if still_due {
            compensate_due_admission(state, &due).await;
        }
    }

    let sessions = state.with_store(|store| store.list_running_sessions().unwrap_or_default());
    for session in sessions {
        let routes = state
            .with_store(|store| store.list_session_routes(&session.pubkey))
            .unwrap_or_default();
        for (channel, _) in routes {
            let member = state
                .with_store(|store| store.get_session_standing(&session.pubkey, &channel))
                .ok()
                .flatten()
                .is_some_and(|standing| standing.state == crate::state::StandingState::Member);
            if member {
                continue;
            }
            repair_one(state, &session, &channel).await;
        }
    }
}

async fn repair_one(state: &Arc<DaemonState>, session: &Session, channel: &str) {
    let _lane = state.standing_sync.lock().await;
    if !admission_is_current(
        state,
        &session.pubkey,
        channel,
        session.runtime_generation,
        session.lifecycle_epoch,
        false,
    ) {
        tracing::debug!(
            pubkey = %session.pubkey,
            %channel,
            "running-standing repair was cancelled because its route is no longer current"
        );
        return;
    }
    let relay_parent = state.with_store(|store| store.channel_parent(channel).ok().flatten());
    let parent = crate::fabric::nip29::readiness::effective_parent_hint(
        relay_parent,
        (!session.readiness_parent.is_empty()).then_some(session.readiness_parent.as_str()),
        channel,
    );
    let confirmed = matches!(
        tokio::time::timeout(
            Duration::from_secs(15),
            state.provider().ensure_channel_ready(
                crate::fabric::nip29::readiness::ChannelCtx {
                    channel,
                    expect_member: &session.pubkey,
                    parent_hint: parent.as_deref(),
                    name: None,
                    repair_whitelisted_admins: true,
                },
            ),
        )
        .await,
        Ok(gate) if gate.is_ready()
    );
    if !confirmed {
        tracing::warn!(pubkey = %session.pubkey, %channel, "running session standing remains retryable");
        return;
    }
    match commit_confirmed_admission(
        state,
        &session.pubkey,
        channel,
        session.runtime_generation,
        session.lifecycle_epoch,
    )
    .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(pubkey = %session.pubkey, %channel, "running-standing repair became stale")
        }
        Err(error) => {
            tracing::error!(pubkey = %session.pubkey, %channel, %error, "running-standing repair persistence failed")
        }
    }
}
