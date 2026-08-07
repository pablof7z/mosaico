//! Serial materialization and asynchronous attachment continuation.

use super::*;

struct Prepared {
    outcome: crate::fabric::MaterializationOutcome,
    hosted: Vec<String>,
    now: u64,
    first_sight: bool,
}

/// Materialize synchronously, then move only attachment network I/O off the
/// relay's global demux loop. Routing for that event remains after its files.
pub(super) fn dispatch(state: &Arc<DaemonState>, event: &Event) {
    let prepared = prepare(state, event);
    if prepared.first_sight && attachments::required(&prepared.outcome) {
        let state = state.clone();
        let event = event.clone();
        tokio::spawn(async move {
            attachments::materialize(&state, &prepared.outcome, &event).await;
            finish(&state, &event, prepared);
        });
    } else {
        finish(state, event, prepared);
    }
}

#[cfg(test)]
pub(super) async fn handle_for_test(state: &Arc<DaemonState>, event: &Event) {
    let prepared = prepare(state, event);
    if prepared.first_sight {
        attachments::materialize(state, &prepared.outcome, event).await;
    }
    finish(state, event, prepared);
}

fn prepare(state: &Arc<DaemonState>, event: &Event) -> Prepared {
    tracing::debug!(
        kind = event.kind.as_u16(),
        id = %&event.id.to_hex()[..8],
        from = %crate::util::pubkey_short(&event.pubkey.to_hex()),
        "incoming event"
    );
    let env = crate::fabric::RawEnvelope::Nostr(event.clone());
    let mut hosted = state.hosted_pubkeys();
    hosted.extend(crate::identity::list_local_pubkeys(
        &crate::config::mosaico_home(),
    ));
    hosted.extend(state.with_store(|s| s.list_local_session_pubkeys().unwrap_or_default()));
    hosted.sort_unstable();
    hosted.dedup();
    let outcome = state.with_store(|s| state.provider().materialize(&env, s));
    // Claim before spawning: duplicate observations never race the file writer.
    let first_sight = state.first_sight(&event.id.to_hex());
    Prepared {
        outcome,
        hosted,
        now: now_secs(),
        first_sight,
    }
}

fn finish(state: &Arc<DaemonState>, event: &Event, prepared: Prepared) {
    super::finish_incoming(
        state,
        event,
        prepared.outcome,
        prepared.hosted,
        prepared.now,
        prepared.first_sight,
    );
}
