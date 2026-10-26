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
/// no other owner closes its Nostr runtime observation here.
pub(in crate::daemon::server) async fn sync_subscriptions(state: &Arc<DaemonState>) -> Result<()> {
    let _serial = state.subscriptions.sync.lock().await;
    let snapshot = build_coverage_snapshot(state);
    // The stable kind:0 profile demand is scoped to the current member set —
    // the backend identity plus every channel's members and admins — which
    // `build_coverage_snapshot` already computes as `profile_pubkeys`. Drive
    // it here so every coverage refresh (startup + every group-state change)
    // re-scopes its one `authors:[members]` filter.
    replace_profile_demand(state, &snapshot.profile_pubkeys).await?;
    // The relay-signed group records ride ONE stable product demand rather than
    // the per-entity refcount, because their branch count is set by the host
    // set and not by how many groups are named.
    let trusted_operators = state.whitelisted_pubkeys();
    super::group_records::sync(
        state,
        super::group_records::GroupRecordsCoverage::from_snapshot(&snapshot, &trusted_operators),
    )
    .await?;
    // Compute the plan under the policy lock, then drop it before handing the
    // effects to the thin Nostr runtime adapter.
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
/// membership-mutation RPCs (leave/session-end) where the teardown is
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

async fn apply_effect(state: &Arc<DaemonState>, effect: &SubEffect) -> Result<()> {
    if let SubEffect::Close { id } = effect {
        tracing::debug!(
            subscription = id,
            "closing Nostr product demand (last owner left)"
        );
    }
    state.nostr().apply(effect).await
}

async fn replace_profile_demand(
    state: &Arc<DaemonState>,
    authors: &BTreeSet<String>,
) -> Result<()> {
    use nostr::prelude::{Filter, Kind, PublicKey};
    const ID: &str = "mosaico-profiles";
    if *state.subscriptions.profile_authors.lock().unwrap() == *authors {
        return Ok(());
    }
    let parsed = authors
        .iter()
        .map(|author| PublicKey::parse(author).map_err(anyhow::Error::from))
        .collect::<Result<Vec<_>>>()?;
    let result = if parsed.is_empty() {
        state.nostr().close_demand(ID).await
    } else {
        state
            .nostr()
            .replace_demand(
                ID,
                vec![Filter::new()
                    .kinds([Kind::Metadata, Kind::EventDeletion])
                    .authors(parsed)
                    .limit(crate::nostr_runtime::PROFILE_EVENT_LIMIT)],
            )
            .await
    };
    if result.is_ok() {
        *state.subscriptions.profile_authors.lock().unwrap() = authors.clone();
    }
    result
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
/// - `profile_pubkeys`: the backend identity plus every current group member.
///   Each gets a narrow exact-author kind:0 observation, so profile reads are
///   answered by current NIP-29 state rather than an app-owned cache.
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
    let mut group_state_channels = BTreeSet::new();
    let backend_pubkey = state.backend_pubkey();

    let archived = state.with_store(|s| {
        // One projection serves every group lookup below; each store accessor
        // would otherwise rebuild it, once per local pubkey and channel.
        let groups = s.group_projection_snapshot();
        let local_pubkeys = s.list_local_session_pubkeys().unwrap_or_default();
        pubkeys.extend(local_pubkeys.iter().cloned());
        pubkeys.insert(backend_pubkey.clone());
        profile_pubkeys.insert(backend_pubkey.clone());
        // Channels any ordinal pubkey is a member of (spawn-on-mention path),
        // plus channels this backend manages as admin; admins are members.
        let mut owners: BTreeSet<String> = local_pubkeys.into_iter().collect();
        owners.insert(backend_pubkey.clone());
        daemon_channels.extend(groups.channels_involving(&owners));
        for (channel_h, roster) in groups.active_channel_rosters() {
            group_state_channels.insert(channel_h.to_string());
            profile_pubkeys.extend(roster.into_iter().map(str::to_string));
        }
        // Channels each live session listens to (active + passively joined).
        for sess in s.list_running_sessions().unwrap_or_default() {
            let joined = s.list_session_routes(&sess.pubkey).unwrap_or_default();
            sessions.insert(
                sess.pubkey.clone(),
                joined.into_iter().map(|(channel, _)| channel).collect(),
            );
        }
        // Archived channels are excluded from all #h/#d coverage. Compute the flag
        // over the union of every candidate channel so the reconciler can subtract.
        let mut candidates: BTreeSet<String> = daemon_channels.clone();
        candidates.extend(group_state_channels.iter().cloned());
        for chans in sessions.values() {
            candidates.extend(chans.iter().cloned());
        }
        let archived_ids = groups.archived_channel_ids();
        candidates
            .into_iter()
            .filter(|channel| archived_ids.contains(channel))
            .collect::<BTreeSet<String>>()
    });

    CoverageSnapshot {
        daemon_channels,
        group_state_channels,
        addressed_pubkeys: pubkeys,
        profile_pubkeys,
        archived_channels: archived,
        sessions,
    }
}
