//! Turn-context assembly shared by the daemon's `turn_start` / `turn_check`
//! RPCs. This is the single source of truth for the text injected into an
//! agent each turn: membership warnings, inbox mentions, ambient chat, and
//! fabric awareness.

use super::reads::{
    ambient_by_joined_channel, context_instance, joined_channels, root_channel_h, take_inbox,
};
use super::TurnContext;
use crate::fabric_context::{capture_inputs, inbox_seed, FabricContextInput};
use crate::state::{Session, Store};
use crate::util::now_secs;

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads that belong to rendering (drain inbox
/// → mark delivered) happen here under the shared store; cursor advancement is
/// applied by the daemon after the graph-derived render.
///
/// `backend_pubkey` is this daemon's signing pubkey, used to decide whether we
/// manage (admin) the channel. `_prev_turn_started_at` is retained for the daemon
/// call contract, but first-turn detection is based on `seen_cursor`: `turn_end`
/// clears `turn_started_at`, while `seen_cursor` is the durable awareness cursor.
/// Text-only shim preserving the historical `Option<String>` contract for the
/// hook-parity tests; the daemon calls [`assemble_turn_start`] for the receipt.
#[cfg(test)]
pub(crate) fn assemble_turn_start_context(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    self_host: &str,
    prev_turn_started_at: u64,
) -> Option<String> {
    let hook_contexts = super::HookContextGraphs::default();
    assemble_turn_start(
        store,
        rec,
        backend_pubkey,
        self_host,
        prev_turn_started_at,
        &hook_contexts,
    )
    .text
}

pub(crate) fn assemble_turn_start(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    self_host: &str,
    _prev_turn_started_at: u64,
    hook_contexts: &super::HookContextGraphs,
) -> TurnContext {
    let first_turn = rec.seen_cursor == 0;
    // Routing scope is the session's `channel_h` — a root channel, or the
    // session/task channel a `channels switch` moved it into. All fabric
    // presence/deltas key on this so a switched session's turn context reflects
    // the channel it actually publishes into.
    let scope = rec.channel_h.clone();
    let self_instance = context_instance(store, rec);
    let self_slug = self_instance.display_slug();
    let self_pubkey = self_instance.pubkey.clone();
    let now = now_secs();
    let mut warnings: Vec<String> = Vec::new();
    let (joined, joined_read_failed) = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };

    if first_turn {
        // Warn only when this daemon does not manage the channel. If it is an
        // admin, channel/room-minting is responsible for signing the member-add
        // itself; a cache miss here is transient local state, not a user action.
        // Compute membership AND the names needed for the warning in one lock.
        let warn = {
            let s = store.lock().expect("store mutex poisoned");
            // A lookup error is NOT membership: treat an Err as "unknown" and
            // fail loud rather than assuming the agent is a member (which would
            // silently suppress the warning when the DB read actually failed).
            let member = match s.is_channel_member(&scope, &self_pubkey) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: channel membership lookup failed; cannot confirm membership"
                    );
                    false
                }
            };
            // Likewise, an admin-lookup error must not be read as "we manage it"
            // — that would suppress a legitimate not-a-member warning.
            let locally_managed = match s.is_channel_admin(&scope, backend_pubkey) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: channel admin lookup failed; cannot confirm management"
                    );
                    false
                }
            };
            (!member && !locally_managed).then(|| {
                let root = root_channel_h(&s, &scope);
                let channel_name = crate::injection::channel_display(&s, &scope);
                let root_name = crate::injection::channel_display(&s, &root);
                (root, channel_name, root_name)
            })
        };
        if let Some((root, channel_name, root_name)) = warn {
            // Name the scope precisely: a channel distinct from its root channel
            // gets both. When the scope IS the root channel, the channel and root
            // coincide and only the root is named.
            let where_label = if root == scope {
                format!("channel \"{root_name}\"")
            } else {
                format!("channel \"{channel_name}\" (in channel \"{root_name}\")")
            };
            warnings.push(format!(
                "WARNING: this agent ({slug}) is not a member of the NIP-29 group \
                 for {where_label}. Messages published by this session may be \
                 rejected by the relay. Ask an operator with relay admin access \
                 to add this agent to the channel.",
                slug = self_slug,
            ));
        }
    }

    // Direct deliveries (p-tagged mentions) come from the inbox ledger. Fabric
    // awareness renders channel chat from the relay-event log:
    //   - First turn: only messages since this session started (pre-join history
    //     is announced as a compact count, not dumped inline).
    //   - Subsequent turns: messages since the last seen_cursor high-water mark.
    // First turn uses session creation time as the ambient floor. Directly injected
    // direct mentions are tracked in the inbox ledger, not by advancing this
    // awareness cursor, so first-turn orientation/pre-history still renders.
    let ambient_since = if first_turn {
        rec.created_at.max(rec.seen_cursor)
    } else {
        rec.seen_cursor
    };
    // Seed with the joined-channel read result: a failure there silently dropped
    // passive channels, so the marker must fire even if every other read succeeds.
    let mut read_failed = joined_read_failed;
    let (mentions, pre_history_notice) = {
        let s = store.lock().expect("store mutex poisoned");
        // A failed inbox claim must NOT render as an empty inbox: log loudly and
        // flag the turn so a visible marker is injected below.
        let mentions = match take_inbox(&s, &rec.session_id, now) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    session = %rec.session_id,
                    error = ?e,
                    "turn_start: inbox claim failed; direct mentions may be dropped"
                );
                read_failed = true;
                Vec::new()
            }
        };
        let (_ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, ambient_since, &self_pubkey);
        read_failed |= ambient_failed;
        let notice = if first_turn {
            // A count failure must not silently render as "no prior history": log
            // loudly and flag the turn so the read-failure marker fires instead of
            // quietly hiding pre-join messages.
            let n = match s.count_channel_events_before(&scope, rec.created_at) {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!(
                        channel = %scope,
                        error = ?e,
                        "turn_start: pre-join history count failed; prior messages may be hidden"
                    );
                    read_failed = true;
                    0
                }
            };
            if n > 0 {
                let name = crate::injection::channel_display(&s, &scope);
                Some(format!(
                    "{n} message(s) in #{name} before you joined this session. \
                     Run `tenex-edge channel read` to see them."
                ))
            } else {
                None
            }
        } else {
            None
        };
        (mentions, notice)
    };
    if read_failed {
        warnings.push(
            "Fabric read failed while assembling this turn; your inbox and/or \
             channel activity below may be incomplete. Do NOT assume the channel \
             is quiet or that you have no mentions."
                .to_string(),
        );
    }
    if let Some(notice) = pre_history_notice {
        warnings.push(notice);
    }

    let forced = mentions.iter().map(inbox_seed).collect::<Vec<_>>();
    // Freeze the canonical inputs from the store, then derive the snapshot in the
    // graph. The reconciler is the single authority that both PRODUCES the injected
    // text and EXPLAINS it (the receipt), so the two cannot drift.
    let inputs = {
        let s = store.lock().expect("store mutex poisoned");
        capture_inputs(
            &s,
            &FabricContextInput {
                session: Some(rec),
                scope: &scope,
                cursor: rec.seen_cursor,
                now,
                self_slug: &self_slug,
                self_pubkey: &self_pubkey,
                backend_pubkey,
                local_host: self_host,
                forced_messages: &forced,
                warnings: &warnings,
                force: false,
            },
        )
    };
    let render_start = std::time::Instant::now();
    let replay_inputs = inputs.clone();
    let outcome = super::render_hook_context(
        hook_contexts,
        &rec.session_id,
        "turn_start",
        rec.seen_cursor as i64,
        now as i64,
        inputs,
    )
    .expect("hook-context snapshot derivation");
    // §4.1: ledger EVERY render, incl. suppressed/no-op ones (which record no
    // receipt) — the hook Unchanged-frame evidence `probe stats` reports.
    {
        let s = store.lock().expect("store mutex poisoned");
        crate::instrument::record_commit(
            &s,
            "hook_context",
            "turn_start",
            Some(&rec.session_id),
            &outcome.commit,
            render_start.elapsed().as_micros() as i64,
            crate::instrument::now_millis(),
        );
    }

    let replay_fact = super::hook_replay_fact(
        &rec.session_id,
        "turn_start",
        rec.seen_cursor as i64,
        now as i64,
        false,
        &replay_inputs,
        outcome.text.as_deref(),
    );
    TurnContext {
        text: outcome.text,
        receipt: outcome.receipt,
        transaction_id: outcome.transaction_id,
        revision: outcome.revision,
        replay_fact,
    }
}
