use super::*;
use crate::reconcile::{CoverageSnapshot, SubEffect};
use std::collections::{BTreeMap, BTreeSet};

/// Record `channel` as an explicitly-subscribed channel and reconcile. The
/// subscribed set feeds daemon-scope coverage; the policy opens a narrow
/// per-entity observation for a newly covered channel and closes observations
/// no longer owned by anyone. An already-covered channel yields no effects.
pub(in crate::daemon::server) async fn ensure_subscription(
    state: &Arc<DaemonState>,
    channel: &str,
) -> Result<()> {
    {
        let mut projs = state.subscriptions.roots.lock().unwrap();
        if !projs.iter().any(|p| p == channel) {
            projs.push(channel.to_string());
        }
    }
    sync_subscriptions(state).await
}

/// Full recompute + apply: build the current coverage snapshot from the store,
/// hand it to the [`SubscriptionReconciler`], and apply the returned
/// Open/Close/Replace effects. This is the single point every subscription
/// writer funnels through — no split-brain. A channel a session just left with
/// no other owner closes its NMP observation here.
pub(in crate::daemon::server) async fn sync_subscriptions(state: &Arc<DaemonState>) -> Result<()> {
    let _serial = state.subscriptions.sync.lock().await;
    let snapshot = build_coverage_snapshot(state);
    // Compute the plan under the policy lock, then drop it before handing the
    // effects to NMP's engine.
    let effects = {
        let mut rec = state.subscriptions.reconciler.lock().unwrap();
        rec.plan(&snapshot)
    };
    for effect in effects {
        apply_effect(state, &effect).await?;
        state
            .subscriptions
            .reconciler
            .lock()
            .unwrap()
            .confirm(&effect);
    }
    Ok(())
}

/// Reconcile subscriptions and log (never propagate) a failure. Used by the
/// membership-mutation RPCs (leave/switch/session-end) where the teardown is
/// best-effort: the store already reflects the change, so a transient relay
/// hiccup must not fail the RPC.
pub(in crate::daemon::server) async fn reconcile_subs_logged(
    state: &Arc<DaemonState>,
    cause: &str,
) {
    if let Err(e) = sync_subscriptions(state).await {
        tracing::warn!(cause, error = %e, "subscription sync failed");
    }
}

pub(in crate::daemon::server) fn reconcile_after_admin_event(
    state: &Arc<DaemonState>,
    event: &nostr::Event,
    first_sight: bool,
) {
    if !first_sight || event.kind.as_u16() != crate::fabric::nip29::wire::KIND_GROUP_ADMINS {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        reconcile_subs_logged(&state, "group admins changed").await;
    });
}

/// Apply policy effects through NMP. NMP owns relay planning, observation
/// lifecycle, reconnect repair, and canonical wire-event deduplication.
pub(in crate::daemon::server) async fn apply_effects(
    state: &Arc<DaemonState>,
    effects: Vec<SubEffect>,
) -> Result<()> {
    for effect in effects {
        apply_effect(state, &effect).await?;
    }
    Ok(())
}

async fn apply_effect(state: &Arc<DaemonState>, effect: &SubEffect) -> Result<()> {
    if let SubEffect::Close { id } = effect {
        tracing::debug!(
            subscription = id,
            "closing NMP observation (last owner left)"
        );
    }
    state.nmp.apply(effect)
}

/// Reopen the channel observation so NMP re-emits its canonical cached rows to a
/// session that became running after a mention was first materialized.
pub(in crate::daemon::server) async fn replay_channel_chat(state: &Arc<DaemonState>, h: &str) {
    tracing::debug!(
        channel = h,
        "replaying channel chat (spawn-on-mention catch-up)"
    );
    let effect = SubEffect::Replace {
        id: format!("mosaico-h-{h}"),
        query: crate::reconcile::SubscriptionQuery {
            kinds: BTreeSet::from([
                crate::fabric::nip29::wire::KIND_CHAT,
                crate::fabric::nip29::wire::KIND_STATUS,
            ]),
            authors: BTreeSet::new(),
            tag: Some(('h', h.to_string())),
        },
    };
    if let Err(error) = apply_effects(state, vec![effect]).await {
        tracing::warn!(channel = h, error = %error, "channel chat replay failed");
    }
}

/// Compute the daemon's current subscription coverage from durable sources,
/// split by owner so channels can refcount per session:
///
/// - `daemon_channels` / archived: explicitly requested daemon observations,
///   groups any local pubkey currently stands in, and groups this daemon
///   manages (admin). Owned by the daemon scope. Session-derived membership is
///   recomputed rather than copied into the explicit set.
/// - `sessions`: each running session mapped to the channels it has joined. Each
///   session is its own scope, so a shared channel stays open until the LAST
///   owning session leaves.
/// - `addressed_pubkeys`: selected session pubkeys and the backend identity.
///   Owned by the daemon scope.
/// - `profile_pubkeys`: the backend identity plus current root-channel admins.
///   Each gets a narrow exact-author kind:0 observation so host snapshots stay
///   current without a global profile feed.
fn build_coverage_snapshot(state: &Arc<DaemonState>) -> CoverageSnapshot {
    let mut daemon_channels: BTreeSet<String> = state
        .subscriptions
        .roots
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let mut pubkeys: BTreeSet<String> = BTreeSet::new();
    let mut profile_pubkeys: BTreeSet<String> = BTreeSet::new();
    let mut sessions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let backend_pubkey = state.backend_pubkey();

    let archived = state.with_store(|s| {
        let local_pubkeys = s.list_local_session_pubkeys().unwrap_or_default();
        pubkeys.extend(local_pubkeys.iter().cloned());
        if let Some(pk) = backend_pubkey.as_ref() {
            pubkeys.insert(pk.clone());
            profile_pubkeys.insert(pk.clone());
        }
        // Channels any ordinal pubkey is a member of (spawn-on-mention path),
        // plus channels this backend manages as admin.
        for pk in local_pubkeys.iter().chain(backend_pubkey.iter()) {
            if let Ok(gs) = s.list_channels_where_member(pk) {
                daemon_channels.extend(gs);
            }
            if let Ok(gs) = s.list_channels_where_admin(pk) {
                daemon_channels.extend(gs);
            }
        }
        for channel in s.list_channels().unwrap_or_default() {
            if channel.is_archived() || !channel.parent.is_empty() {
                continue;
            }
            profile_pubkeys.extend(
                s.list_channel_members(&channel.channel_h)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|member| member.role == "admin")
                    .map(|member| member.pubkey),
            );
        }
        // Channels each live session listens to (active + passively joined).
        for sess in s.list_running_sessions().unwrap_or_default() {
            let joined = s
                .list_session_routes(&sess.pubkey)
                .unwrap_or_else(|_| vec![(sess.channel_h.clone(), sess.created_at)]);
            sessions.insert(
                sess.pubkey.clone(),
                joined.into_iter().map(|(channel, _)| channel).collect(),
            );
        }
        // Archived channels are excluded from all #h/#d coverage. Compute the flag
        // over the union of every candidate channel so the reconciler can subtract.
        let mut candidates: BTreeSet<String> = daemon_channels.clone();
        for chans in sessions.values() {
            candidates.extend(chans.iter().cloned());
        }
        candidates
            .into_iter()
            .filter(|channel| s.is_archived_channel(channel).unwrap_or(false))
            .collect::<BTreeSet<String>>()
    });

    CoverageSnapshot {
        daemon_channels,
        addressed_pubkeys: pubkeys,
        profile_pubkeys,
        archived_channels: archived,
        sessions,
    }
}
