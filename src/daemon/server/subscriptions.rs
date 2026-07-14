use super::*;
use crate::fabric::subscriptions::{id_h_narrow, narrow_h_filter};
use crate::reconcile::{CoverageSnapshot, InputFact, SubEffect};
use std::collections::{BTreeMap, BTreeSet};

/// Record `channel` as an explicitly-subscribed channel and reconcile. The
/// subscribed set feeds the daemon-scope coverage; the reconciler opens a narrow
/// per-entity REQ for any newly-covered channel and, critically, CLOSES any REQ
/// no longer owned by anyone. Idempotent: an already-covered channel yields no
/// effects. Bounded — opening/closing a relay REQ can hang on a slow relay, and
/// this is awaited on hook-critical paths (session_start, spawn_session), so the
/// intent (channel recorded above + folded into the reconciler) survives a
/// timeout; we fail open.
pub(in crate::daemon::server) async fn ensure_subscription(
    state: &Arc<DaemonState>,
    channel: &str,
) -> Result<()> {
    {
        let mut projs = state.subscribed_root_channels.lock().unwrap();
        if !projs.iter().any(|p| p == channel) {
            projs.push(channel.to_string());
        }
    }
    match tokio::time::timeout(std::time::Duration::from_secs(5), sync_subscriptions(state)).await {
        Ok(r) => r,
        Err(_) => {
            tracing::warn!(
                channel = channel,
                "subscription sync timed out; continuing best-effort"
            );
            Ok(())
        }
    }
}

/// Full recompute + apply: build the current coverage snapshot from the store,
/// hand it to the [`SubscriptionReconciler`], and apply the returned
/// Open/Close/Replace effects. This is the single point every subscription
/// writer funnels through — no split-brain. Replaces the old `resubscribe`
/// (aggregate seed) AND adds the previously-missing teardown path: a channel a
/// session just left with no other owner produces a real NIP-01 CLOSE here.
pub(in crate::daemon::server) async fn sync_subscriptions(state: &Arc<DaemonState>) -> Result<()> {
    let snapshot = build_coverage_snapshot(state);
    // Compute the plan under the lock, then DROP the guard before any await: the
    // reconciler is a plain Mutex (never held across `.await`), the transport does
    // the network I/O.
    let start = std::time::Instant::now();
    let (effects, result, facts, preview) = {
        let mut rec = state.subs.lock().unwrap();
        let preview = rec
            .preview_sync(&snapshot)
            .map_err(|e| anyhow::anyhow!("subscription preview failed: {e:?}"))?
            .result;
        let (effects, result) = rec
            .sync(&snapshot)
            .map_err(|e| anyhow::anyhow!("subscription reconcile failed: {e:?}"))?;
        // Flatten EVERY commit (incl. no-op recomputes) through the surface labels.
        let mut facts = crate::reconcile::CommitFacts::from_result(
            rec.labels(),
            &result,
            rec.graph_node_count(),
        );
        facts.graph_resources = rec.state_rows().len() as i64;
        (effects, result, facts, preview)
    };
    let duration_us = start.elapsed().as_micros() as i64;
    // §4.1: record the all-commit ledger row for EVERY sync, incl. no-ops (which
    // record no receipt) — the suppression evidence `probe stats` reports.
    state.with_store(|s| {
        let created_at = crate::instrument::now_millis();
        crate::instrument::record_commit(
            s,
            "subscriptions",
            "sync",
            None,
            &facts,
            duration_us,
            created_at,
        );
        crate::replay_capsules::record(
            s,
            "subscriptions",
            "sync",
            None,
            InputFact::SubscriptionSync {
                snapshot: snapshot.clone(),
                at: created_at.max(0) as u64 / 1000,
            },
            created_at,
        );
    });
    // Slice 8: record the drive-seam receipt (host-side, off the graph path) only
    // when the sync actually opened/closed a REQ — a no-op recompute leaves no noise.
    if !effects.is_empty() {
        let row = crate::state::receipts::NewReceipt {
            surface: "subscriptions".into(),
            transaction_id: result.transaction_id.get() as i64,
            revision: result.revision.get() as i64,
            changed_summary: crate::instrument::changed_summary_json(
                &result.changed_inputs,
                &result.changed_derived_nodes,
                &result.changed_collection_nodes,
                None,
                None,
            ),
            commands: crate::instrument::commands_json(result.resource_plan.commands()),
            artifact_ref: None,
            created_at: crate::instrument::now_millis(),
        };
        state.with_store(|s| crate::instrument::record_receipt(s, row));
    }
    if !effects.is_empty() && !preview_matches(&preview, &result) {
        return Err(anyhow::anyhow!(
            "subscription effects blocked: committed plan was not previewed first"
        ));
    }
    apply_effects(state, effects, &preview).await
}

/// `resubscribe` is now a thin alias for the reconciler sync, kept for the
/// startup + reconcile call sites that name it.
pub(in crate::daemon::server) async fn resubscribe(state: &Arc<DaemonState>) -> Result<()> {
    sync_subscriptions(state).await
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

/// Apply the reconciler's host effects on the MAIN relays only. Open/Replace both
/// re-`subscribe_with_id_to` under the entity's semantic id (NIP-01
/// replace-in-place); Close sends a real NIP-01 CLOSE. Broad filters never hit
/// the kind:0 indexer relay — these are narrow per-entity REQs and profiles
/// resolve on-demand.
pub(in crate::daemon::server) async fn apply_effects(
    state: &Arc<DaemonState>,
    effects: Vec<SubEffect>,
    _preview: &trellis_core::TransactionResult<crate::reconcile::SubCommand>,
) -> Result<()> {
    for effect in effects {
        match effect {
            SubEffect::Open { id, filter } | SubEffect::Replace { id, filter } => {
                state
                    .transport
                    .subscribe_with_id_to(&state.cfg.relays, id, filter)
                    .await?;
            }
            SubEffect::Close { id } => {
                tracing::debug!(subscription = %id, "closing relay REQ (last owner left)");
                state.transport.unsubscribe(&id).await?;
            }
        }
    }
    Ok(())
}

/// Force the relay to replay channel `h`'s stored chat so a session that just
/// became alive receives messages published BEFORE it existed (the spawn-on-
/// mention case: the triggering kind:9 arrives, spawns the agent, but the live
/// materialize path can only route to sessions already alive). Re-applying the
/// channel's narrow `#h` REQ under its stable per-entity id replaces it in place
/// (NIP-01) and the relay re-streams the stored events, which
/// `materialize_chat_message` then routes to the now-alive session. Best-effort
/// and bounded so a slow relay can't block the hook.
pub(in crate::daemon::server) async fn replay_channel_chat(state: &Arc<DaemonState>, h: &str) {
    tracing::debug!(
        channel = h,
        "replaying channel chat (spawn-on-mention catch-up)"
    );
    let effect = SubEffect::Replace {
        id: id_h_narrow(h),
        filter: narrow_h_filter(h),
    };
    let snapshot = build_coverage_snapshot(state);
    let preview = {
        let mut rec = state.subs.lock().unwrap();
        match rec.preview_sync(&snapshot) {
            Ok(preview) => preview.result,
            Err(e) => {
                tracing::warn!(channel = h, error = ?e, "channel chat replay skipped: preview failed");
                return;
            }
        }
    };
    let fut = apply_effects(state, vec![effect], &preview);
    if tokio::time::timeout(std::time::Duration::from_secs(5), fut)
        .await
        .is_err()
    {
        tracing::warn!(channel = h, "channel chat replay timed out (best-effort)");
    }
}

fn preview_matches(
    preview: &trellis_core::TransactionResult<crate::reconcile::SubCommand>,
    committed: &trellis_core::TransactionResult<crate::reconcile::SubCommand>,
) -> bool {
    preview.revision == committed.revision
        && crate::reconcile::preview::command_plans_match(
            preview.resource_plan.commands(),
            committed.resource_plan.commands(),
        )
}

/// Compute the daemon's current subscription coverage from durable sources,
/// split by owner so channels can refcount per session:
///
/// - `daemon_channels` / archived: explicitly tracked channels, groups any
///   local session pubkey is a member of (spawn-on-mention), and groups this
///   daemon manages (admin). Owned by the daemon scope.
/// - `sessions`: each alive session mapped to the channels it has joined. Each
///   session is its own scope, so a shared channel stays open until the LAST
///   owning session leaves.
/// - `addressed_pubkeys`: selected session pubkeys and the backend identity.
///   Owned by the daemon scope.
fn build_coverage_snapshot(state: &Arc<DaemonState>) -> CoverageSnapshot {
    let mut daemon_channels: BTreeSet<String> = state
        .subscribed_root_channels
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let mut pubkeys: BTreeSet<String> = BTreeSet::new();
    let mut sessions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let backend_pubkey = state.backend_pubkey();

    let archived = state.with_store(|s| {
        let local_pubkeys = s.list_local_session_pubkeys().unwrap_or_default();
        pubkeys.extend(local_pubkeys.iter().cloned());
        if let Some(pk) = backend_pubkey.as_ref() {
            pubkeys.insert(pk.clone());
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
        // Channels each live session listens to (active + passively joined).
        for sess in s.list_alive_sessions().unwrap_or_default() {
            let joined = s
                .list_session_joined_channels(&sess.pubkey)
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
        archived_channels: archived,
        sessions,
    }
}
