//! Serial product decoding and asynchronous attachment continuation.

use super::*;

struct Prepared {
    decoded: crate::fabric::ProductDecode,
    now: u64,
}

/// Decode synchronously, then move only attachment network I/O off the
/// relay's global demux loop. Routing for that event remains after its files.
pub(super) fn dispatch(state: &Arc<DaemonState>, event: &Event) {
    let prepared = prepare(state, event);
    if attachments::required(&prepared.decoded) {
        let state = state.clone();
        let event = event.clone();
        tokio::spawn(async move {
            attachments::materialize(&state, &prepared.decoded, &event).await;
            finish(&state, &event, prepared);
        });
    } else {
        finish(state, event, prepared);
    }
}

#[cfg(test)]
pub(super) async fn handle_for_test(state: &Arc<DaemonState>, event: &Event) {
    let prepared = prepare(state, event);
    attachments::materialize(state, &prepared.decoded, event).await;
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
    let decoded = state.with_store(|s| state.provider().decode_product_event(&env, s));
    Prepared {
        decoded,
        now: now_secs(),
    }
}

fn finish(state: &Arc<DaemonState>, event: &Event, prepared: Prepared) {
    super::finish_incoming(state, event, prepared.decoded, prepared.now);
}
