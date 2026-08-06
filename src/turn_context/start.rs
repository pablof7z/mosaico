//! Turn-context assembly shared by the daemon's `turn_start` / `turn_check`
//! RPCs. This is the single source of truth for the text injected into an
//! agent each turn: membership warnings, inbox mentions, ambient chat, and
//! fabric awareness.

use super::reads::{ambient_by_joined_channel, context_instance, joined_channels, take_inbox};
use super::TurnContext;
use crate::fabric_context::{capture_inputs, inbox_seed, FabricContextInput};
use crate::state::{Session, Store};
use crate::util::now_secs;
use anyhow::Result;

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads that belong to rendering (drain inbox
/// → mark delivered) happen here under the shared store; cursor advancement is
/// applied by the daemon after the context render.
///
/// `backend_pubkey` is this daemon's signing pubkey, used to decide whether we
/// manage (admin) the channel. `_prev_turn_started_at` is retained for the daemon
/// call contract, but first-turn detection is based on `seen_cursor`: `turn_end`
/// clears `turn_started_at`, while `seen_cursor` is the durable awareness cursor.
/// Test helper that returns only the agent-visible text. Production callers use
/// [`assemble_turn_start`] and retain the accompanying receipt.
#[cfg(test)]
pub(crate) fn render_turn_start_text_for_test(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    self_host: &str,
    prev_turn_started_at: u64,
) -> Option<String> {
    let hook_contexts = super::HookContextStates::default();
    assemble_turn_start(
        store,
        rec,
        backend_pubkey,
        self_host,
        prev_turn_started_at,
        &hook_contexts,
    )
    .unwrap()
    .text
}

pub(crate) fn assemble_turn_start(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    backend_pubkey: &str,
    self_host: &str,
    _prev_turn_started_at: u64,
    hook_contexts: &super::HookContextStates,
) -> Result<TurnContext> {
    let first_turn = rec.seen_cursor == 0;
    // A session has no mutable current channel. The renderer reads the complete
    // membership set; forced deliveries retain their own envelope channel.
    let scope = String::new();
    let self_instance = context_instance(store, rec);
    let self_slug = self_instance.display_slug();
    let self_pubkey = self_instance.pubkey.clone();
    let now = now_secs();
    let mut warnings: Vec<String> = Vec::new();
    if first_turn {
        warnings
            .push("You must load the `mosaico` skill right away, before continuing.".to_string());
    }
    super::headless::push_mode_notice(store, hook_contexts, rec, true, &mut warnings);

    let (joined, joined_read_failed) = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };

    if first_turn && joined.is_empty() {
        warnings.push(
            "This session has not joined any Mosaico channels. Its launch workspace, \
             current working directory, and normal filesystem access are unchanged."
                .to_string(),
        );
    } else if first_turn {
        // Warn independently for every joined channel whose relay standing has
        // not materialized. No membership is privileged as a current location.
        let missing_relay_memberships = {
            let s = store.lock().expect("store mutex poisoned");
            joined
                .iter()
                .filter_map(|(channel, _)| {
                    let member = s
                        .is_channel_member(channel, &self_pubkey)
                        .unwrap_or_else(|e| {
                            tracing::error!(
                                channel,
                                error = ?e,
                                "turn_start: channel membership lookup failed"
                            );
                            false
                        });
                    let locally_managed = s
                        .is_channel_admin(channel, backend_pubkey)
                        .unwrap_or_else(|e| {
                            tracing::error!(
                                channel,
                                error = ?e,
                                "turn_start: channel admin lookup failed"
                            );
                            false
                        });
                    (!member && !locally_managed)
                        .then(|| crate::channel_ref::full_channel_ref(&s, channel))
                })
                .collect::<Vec<_>>()
        };
        for channel_ref in missing_relay_memberships {
            warnings.push(format!(
                "WARNING: this agent ({self_slug}) has joined {channel_ref} locally, but \
                 its relay membership is not confirmed. Messages may be rejected until \
                 an operator with relay admin access adds this agent."
            ));
        }
    }

    if first_turn {
        let history_notices = {
            let s = store.lock().expect("store mutex poisoned");
            match super::history::prejoin_notices(&s, rec, &joined, now) {
                Ok(notices) => notices,
                Err(error) => {
                    tracing::error!(
                        pubkey = %rec.pubkey,
                        %error,
                        "turn_start: pre-join history summary failed"
                    );
                    warnings.push(
                        "Fabric history summary failed; prior channel activity may be hidden."
                            .to_string(),
                    );
                    Vec::new()
                }
            }
        };
        warnings.extend(history_notices);
    }

    if first_turn {
        // Missing admission is a stable session fact. A hosted endpoint that is
        // temporarily unavailable has a different recovery path and must not be
        // mislabeled as unhosted.
        if rec.admitted_transport.is_empty() {
            warnings.push(
                "This session is unhosted. After this turn ends, later mentions will queue but \
                 cannot start another turn. A pending wait can keep the current invocation \
                 reachable. Read `~/.agents/skills/mosaico/references/unhosted.md` for risks \
                 and mitigations."
                    .to_string(),
            );
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
    let mentions = {
        let s = store.lock().expect("store mutex poisoned");
        // A failed inbox claim must NOT render as an empty inbox: log loudly and
        // flag the turn so a visible marker is injected below.
        let mentions = match take_inbox(&s, &rec.pubkey, now) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    pubkey = %rec.pubkey,
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
        mentions
    };
    if read_failed {
        warnings.push(
            "Fabric read failed while assembling this turn; your inbox and/or \
             channel activity below may be incomplete. Do NOT assume the channel \
             is quiet or that you have no mentions."
                .to_string(),
        );
    }
    let forced = mentions.iter().map(inbox_seed).collect::<Vec<_>>();
    // Freeze the canonical inputs from the store, then render the snapshot. The
    // stateful renderer is the single authority that both produces the injected
    // text and explains it, so the two cannot drift.
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
    }?;
    let missing_profiles = crate::fabric_context::missing_profile_pubkeys(&inputs);
    let outcome = super::render_hook_context(
        hook_contexts,
        &rec.pubkey,
        "turn_start",
        rec.seen_cursor as i64,
        now as i64,
        inputs,
    );
    Ok(TurnContext {
        text: outcome.text,
        receipt: outcome.receipt,
        transaction_id: outcome.transaction_id,
        revision: outcome.revision,
        missing_profiles,
    })
}
