//! Event-driven lifecycle-to-presence publication.

use super::*;

pub(crate) async fn reconcile_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    trigger: &'static str,
    confirmed_scope: Option<crate::fabric::provider::ConfirmedGroupScope>,
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
    let extension_delivery_live = super::session_delivery::extension_delivery_live(state, &session);
    let projection = state.with_store(|store| {
        crate::session_presence::publication_with_extension(
            store,
            &session,
            extension_delivery_live,
        )
    });
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
        crate::presence_publisher::DriveMeta {
            trigger,
            confirmed_scope,
        },
        |status| status.reconcile(pubkey, generation, projection, now_secs()),
    );
}

pub(crate) async fn reassert_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    trigger: &'static str,
    confirmed_scope: Option<crate::fabric::provider::ConfirmedGroupScope>,
) {
    let session = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(session)) if session.runtime_generation == generation && session.is_running() => {
            session
        }
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence reassertion read failed");
            return;
        }
    };
    let extension_delivery_live = super::session_delivery::extension_delivery_live(state, &session);
    let projection = state.with_store(|store| {
        crate::session_presence::publication_with_extension(
            store,
            &session,
            extension_delivery_live,
        )
    });
    let keys = match state.session_signing_keys(pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence reassertion signer unavailable");
            return;
        }
    };
    crate::presence_publisher::drive(
        &state.reconcilers.status,
        &state.reconcilers.presence_publisher,
        &keys,
        crate::presence_publisher::DriveMeta {
            trigger,
            confirmed_scope,
        },
        |status| status.reassert(pubkey, generation, projection, now_secs()),
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
        crate::presence_publisher::DriveMeta {
            trigger,
            confirmed_scope: None,
        },
        |status| status.close(pubkey, generation, at),
    );
}

/// Withdraw this generation's presence from `channel` ahead of leaving it,
/// and wait for the relay's answer.
///
/// A NIP-29 relay refuses a non-member's write to a closed group, so the
/// expiring (or narrowed) status must land while the session is still a
/// member; once the removal is confirmed no status can reach that channel.
pub(super) async fn withdraw_before_leave(
    state: &Arc<DaemonState>,
    pubkey: &str,
    generation: u64,
    channel: &str,
    trigger: &'static str,
) {
    let session = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(session)) if session.runtime_generation == generation && session.is_running() => {
            session
        }
        Ok(_) => return,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence withdrawal read failed");
            return;
        }
    };
    let extension_delivery_live = super::session_delivery::extension_delivery_live(state, &session);
    let mut projection = state.with_store(|store| {
        crate::session_presence::publication_with_extension(
            store,
            &session,
            extension_delivery_live,
        )
    });
    projection.channels.remove(channel);
    let keys = match state.session_signing_keys(pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey, generation, trigger, %error, "presence signer unavailable");
            return;
        }
    };
    let outcome = state
        .reconcilers
        .status
        .lock()
        .expect("status policy poisoned")
        .reconcile(pubkey, generation, projection, now_secs());
    state
        .reconcilers
        .presence_publisher
        .publish_now(outcome, &keys, trigger, None)
        .await;
}
