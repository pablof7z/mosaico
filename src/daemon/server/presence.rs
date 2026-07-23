//! Event-driven lifecycle-to-presence publication.

use super::*;

pub(crate) async fn reconcile_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    trigger: &'static str,
) {
    let session = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(session)) => session,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence projection read failed");
            return;
        }
    };
    if session.runtime_generation != generation || !session.is_running() {
        return;
    }
    let projection =
        state.with_store(|store| crate::session_presence::publication(store, &session));
    let keys = match state.session_signing_keys(pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence signer unavailable");
            return;
        }
    };
    crate::presence_publisher::drive(
        &state.reconcilers.status,
        &state.reconcilers.presence_publisher,
        &keys,
        crate::presence_publisher::DriveMeta { trigger },
        |status| status.reconcile(pubkey, generation, projection, now_secs()),
    );
}

pub(super) async fn close_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    at: u64,
    trigger: &'static str,
) {
    let keys = match state.session_signing_keys(pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence signer unavailable");
            return;
        }
    };
    crate::presence_publisher::drive(
        &state.reconcilers.status,
        &state.reconcilers.presence_publisher,
        &keys,
        crate::presence_publisher::DriveMeta { trigger },
        |status| status.close(pubkey, generation, at),
    );
}
