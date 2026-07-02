use super::*;

pub(in crate::daemon::server) async fn ensure_subscription(
    state: &Arc<DaemonState>,
    project: &str,
) -> Result<()> {
    {
        let mut projs = state.subscribed_projects.lock().unwrap();
        if !projs.iter().any(|p| p == project) {
            projs.push(project.to_string());
        }
    }
    // Incremental add: plan only the NARROW deltas for this newly-tracked channel
    // (one `#h` chat/status/long-form REQ + one group-state REQ), NOT a full
    // aggregate rebuild. Mutating an aggregate makes the relay replay every stored
    // event for every tracked entity; a narrow REQ scoped to just this channel
    // avoids that. The deltas are empty when the channel is already covered (by an
    // aggregate seeded at startup or an earlier narrow add), making this idempotent.
    let reqs = state.subscriptions.lock().unwrap().add_channel(project);
    if reqs.is_empty() {
        return Ok(());
    }
    tracing::debug!(
        channel = project,
        req_count = reqs.len(),
        "opening narrow channel REQs"
    );
    // Bounded: opening a relay subscription can hang on a slow/unreachable relay,
    // and this is awaited on hook-critical paths (session_start, spawn_session),
    // so a hang would block the editor. The intent (project recorded above +
    // folded into the registry) survives a timeout; we fail open.
    match tokio::time::timeout(std::time::Duration::from_secs(5), apply_plan(state, reqs)).await {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!(
                channel = project,
                "subscription apply timed out; continuing best-effort"
            );
            Ok(())
        }
    }
}

/// Open each planned REQ under its semantic [`SubscriptionId`], on the MAIN
/// relays only. Broad `#h`/`#p` aggregate filters must NOT hit the kind:0 indexer
/// relay — that relay is a one-shot profile-resolution endpoint, and pinning a
/// firehose there wastes its connection and pulls in noise. Re-applying the same
/// id REPLACES the relay-side REQ in place (NIP-01), which is exactly how `seed`
/// compacts the three aggregates.
pub(in crate::daemon::server) async fn apply_plan(
    state: &Arc<DaemonState>,
    reqs: Vec<crate::fabric::subscriptions::PlannedReq>,
) -> Result<()> {
    for req in reqs {
        state
            .transport
            .subscribe_with_id_to(&state.cfg.relays, req.id, req.filter)
            .await?;
    }
    Ok(())
}

/// Force the relay to replay channel `h`'s stored chat so a session that just
/// became alive receives messages published BEFORE it existed (the spawn-on-
/// mention case: the triggering kind:9 arrives, spawns the agent, but the live
/// materialize path can only route to sessions already alive). Re-applying the
/// channel's narrow `#h` REQ replaces it in place (NIP-01) and the relay
/// re-streams the stored events, which `materialize_chat_message` then routes to
/// the now-alive session. Best-effort: a replay failure just means the session
/// relies on subsequent live chat. Bounded so a slow relay can't block the hook.
pub(in crate::daemon::server) async fn replay_channel_chat(state: &Arc<DaemonState>, h: &str) {
    tracing::debug!(
        channel = h,
        "replaying channel chat (spawn-on-mention catch-up)"
    );
    let req = crate::fabric::subscriptions::channel_chat_replay_req(h);
    let fut = apply_plan(state, vec![req]);
    if tokio::time::timeout(std::time::Duration::from_secs(5), fut)
        .await
        .is_err()
    {
        tracing::warn!(channel = h, "channel chat replay timed out (best-effort)");
    }
}

/// Close each subscription id (NIP-01 CLOSE). Used when compaction retires the
/// narrow REQs now subsumed by a rebuilt aggregate. Best-effort per id.
#[allow(dead_code)]
pub(in crate::daemon::server) async fn close_subs(
    state: &Arc<DaemonState>,
    ids: Vec<nostr_sdk::prelude::SubscriptionId>,
) -> Result<()> {
    for id in ids {
        state.transport.unsubscribe(&id).await?;
    }
    Ok(())
}

/// Compute the daemon's current subscription coverage from durable sources.
///
/// - `channels_h` / `group_state_d`: explicitly tracked projects, channels live
///   sessions route under, groups any local/ordinal pubkey is a member of, and
///   groups this daemon owns.
/// - `addressed_pubkeys_p`: local durable + ordinal pubkeys, live transient
///   session keys, and the backend identity (folds in the old standalone backend
///   orchestration `#p` subscription).
fn build_entity_coverage(state: &Arc<DaemonState>) -> crate::fabric::subscriptions::EntityCoverage {
    use std::collections::BTreeSet;

    let edge = crate::config::edge_home();
    let local_pks = crate::identity::list_local_pubkeys(&edge);

    let mut channels: BTreeSet<String> = state
        .subscribed_projects
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let mut pubkeys: BTreeSet<String> = local_pks.iter().cloned().collect();

    state.with_store(|s| {
        let ordinals = s.list_identity_pubkeys().unwrap_or_default();
        pubkeys.extend(ordinals.iter().cloned());
        // Channels any local/ordinal pubkey is a member of (spawn-on-mention path),
        // plus channels it manages (admin = the old "owned groups").
        for pk in local_pks.iter().chain(ordinals.iter()) {
            if let Ok(gs) = s.list_channels_where_member(pk) {
                channels.extend(gs);
            }
            if let Ok(gs) = s.list_channels_where_admin(pk) {
                channels.extend(gs);
            }
        }
        // Channels live sessions listen to. This includes the active publishing
        // channel plus any passively joined channels.
        for sess in s.list_alive_sessions().unwrap_or_default() {
            let joined = s
                .list_session_joined_channels(&sess.session_id)
                .unwrap_or_else(|_| vec![(sess.channel_h.clone(), sess.created_at)]);
            for (channel, _) in joined {
                channels.insert(channel);
            }
        }
    });

    // Live transient session keys + backend identity round out the addressed set.
    pubkeys.extend(state.live_session_pubkeys());
    if let Some(bp) = state.backend_pubkey() {
        pubkeys.insert(bp.to_string());
    }

    crate::fabric::subscriptions::EntityCoverage {
        channels_h: channels.clone(),
        group_state_d: channels,
        addressed_pubkeys_p: pubkeys,
    }
}

/// Seed the THREE stable aggregate REQs from the daemon's current coverage. This
/// REPLACES the whole aggregate (the compaction point) and applies exactly three
/// REQs: `#h` (chat/status/long-form over all channels), `#p` (chat/long-form
/// addressed to all durable pubkeys), and group-state (39000/39001/39002 over all
/// group ids). It NO LONGER expands per-(project×kind) `Scope`s and NEVER
/// subscribes kind:0 — profile resolution stays on the on-demand `Transport::fetch`
/// + `profile.rs` cache.
///
/// An aggregate filter with an EMPTY coverage set degenerates to an unscoped
/// firehose over its kinds; such a REQ is skipped (never opened) so a daemon with
/// no channels/pubkeys yet does not pull the whole relay. The registry is still
/// seeded so later narrow adds dedup correctly against the (empty) aggregate.
pub(in crate::daemon::server) async fn resubscribe(state: &Arc<DaemonState>) -> Result<()> {
    let coverage = build_entity_coverage(state);
    // seed() returns the three aggregate REQs in the fixed, tested order
    // [`#h`, `#p`, group-state]; pair each with its set's emptiness so we drop
    // any that would be an unscoped firehose.
    let empties = [
        coverage.channels_h.is_empty(),
        coverage.addressed_pubkeys_p.is_empty(),
        coverage.group_state_d.is_empty(),
    ];
    let reqs = state.subscriptions.lock().unwrap().seed(coverage);
    let reqs: Vec<_> = reqs
        .into_iter()
        .zip(empties)
        .filter_map(|(req, empty)| (!empty).then_some(req))
        .collect();
    apply_plan(state, reqs).await
}
