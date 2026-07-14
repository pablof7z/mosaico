use crate::fabric_context::{capture_inputs, inbox_seed, FabricContextInput};
use crate::state::{Session, Store};

use super::reads::{ambient_by_joined_channel, context_instance, joined_channels, take_inbox};
use super::TurnContext;

/// Text-only shim preserving the historical `Option<String>` contract for the
/// hook-parity tests; the daemon calls [`assemble_turn_check`] for the receipt.
#[cfg(test)]
pub(crate) fn assemble_turn_check_context(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    self_host: &str,
    delta_since: Option<u64>,
    now: u64,
) -> Option<String> {
    let hook_contexts = super::HookContextGraphs::default();
    assemble_turn_check(store, rec, self_host, delta_since, now, &hook_contexts).text
}

/// Mid-turn context for the PostToolUse `turn_check` hook.
pub(crate) fn assemble_turn_check(
    store: &std::sync::Mutex<Store>,
    rec: &Session,
    self_host: &str,
    delta_since: Option<u64>,
    now: u64,
    hook_contexts: &super::HookContextGraphs,
) -> TurnContext {
    let mut warnings: Vec<String> = Vec::new();
    super::headless::push_mode_notice(hook_contexts, store, rec, false, &mut warnings);
    let scope = rec.channel_h.clone();
    let self_instance = context_instance(store, rec);
    let self_slug = self_instance.display_slug();
    let self_pubkey = self_instance.pubkey;
    let (joined, joined_read_failed) = {
        let s = store.lock().expect("store mutex poisoned");
        joined_channels(&s, rec)
    };

    let mut read_failed = joined_read_failed;
    let direct_mentions = {
        let s = store.lock().expect("store mutex poisoned");
        match take_inbox(&s, &rec.pubkey, now) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    pubkey = %rec.pubkey,
                    error = ?e,
                    "turn_check: inbox claim failed; direct mentions may be dropped"
                );
                read_failed = true;
                Vec::new()
            }
        }
    };
    if let Some(since) = delta_since {
        let s = store.lock().expect("store mutex poisoned");
        let (_ambient, ambient_failed) =
            ambient_by_joined_channel(&s, &joined, since, &self_pubkey);
        read_failed |= ambient_failed;
    }

    if read_failed {
        warnings.push(
            "Fabric read failed mid-turn; mentions and/or channel activity below \
             may be incomplete."
                .to_string(),
        );
    }

    let forced = direct_mentions.iter().map(inbox_seed).collect::<Vec<_>>();
    let cursor = delta_since.unwrap_or(now);
    // Always derive through the graph so the receipt is produced even when the
    // snapshot is empty; the empty-view gate suppresses the injected text exactly
    // as the old early-return did (no delta, no mentions, no warnings → None).
    let inputs = {
        let s = store.lock().expect("store mutex poisoned");
        capture_inputs(
            &s,
            &FabricContextInput {
                session: Some(rec),
                scope: &scope,
                cursor,
                now,
                self_slug: &self_slug,
                self_pubkey: &self_pubkey,
                // turn_check is always a delta render (cursor > 0), which never emits
                // a full <members> roster, so the backend-exclusion input is unused
                // here — the roster leak can only surface on a full (cursor == 0) render.
                backend_pubkey: "",
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
        &rec.pubkey,
        "turn_check",
        cursor as i64,
        now as i64,
        inputs,
    )
    .expect("hook-context snapshot derivation");
    // §4.1: ledger EVERY render, incl. suppressed/no-op ones.
    {
        let s = store.lock().expect("store mutex poisoned");
        crate::instrument::record_commit(
            &s,
            "hook_context",
            "turn_check",
            Some(&rec.pubkey),
            &outcome.commit,
            render_start.elapsed().as_micros() as i64,
            crate::instrument::now_millis(),
        );
    }
    let replay_fact = super::hook_replay_fact(
        &rec.pubkey,
        "turn_check",
        cursor as i64,
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
