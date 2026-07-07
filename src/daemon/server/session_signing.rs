use super::*;

/// Select the durable ordinal identity for a session (issue #47).
///
/// `base_keys`/`base_pubkey` are a local derivation root for ordinal identities.
/// The allocator picks ordinal 1 for the first live session in a channel and the
/// next free ordinal for same-channel concurrency. The same ordinal key may be
/// reused by another live session in a different channel.
///
/// Persists the derived signing key into the `identities` cache, binding the
/// ordinal pubkey to this live session + its harness-native id (the resume key)
/// so a later mention can resume the right session.
#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn select_session_signer(
    state: &Arc<DaemonState>,
    session_id: &str,
    base_keys: &Keys,
    base_pubkey: &str,
    agent_slug: &str,
    h: &str,
    native_id: &str,
    hint_ordinal: Option<u32>,
) -> Result<session_signer::SelectedSigner> {
    // Honor (in priority order): an explicit spawn hint (mention-driven exact
    // ordinal), then a session's already-bound ordinal (reassert / restart), so
    // its durable identity survives.
    let existing_identity = state.with_store(|s| s.identity_for_session(session_id).ok().flatten());
    let preferred = hint_ordinal.filter(|n| *n > 0).or_else(|| {
        existing_identity
            .as_ref()
            .map(|i| i.ordinal)
            .filter(|n| *n > 0)
    });
    let preferred_pubkey = preferred.map(|ordinal| {
        crate::identity::derive_agent_ordinal_keys(base_keys, ordinal)
            .public_key()
            .to_hex()
    });
    let occupied_pubkeys: std::collections::HashSet<String> = state.with_store(|s| {
        let mut occupied: std::collections::HashSet<String> = s
            .list_channel_members(h)
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.pubkey)
            .collect();
        for claim in s
            .list_active_session_claims_for_channel(h, now_secs())
            .unwrap_or_default()
        {
            occupied.insert(claim.pubkey);
        }
        occupied
    });

    let signer = {
        let mut reservations = state.session_signers.lock().unwrap();
        let mut session_keys = state.session_keys.lock().unwrap();
        session_signer::select_and_reserve(
            &mut reservations,
            &mut session_keys,
            session_signer::SignerRequest {
                session_id,
                base_pubkey,
                agent_slug,
                h,
                base_keys,
                preferred_ordinal: preferred,
                occupied_pubkeys: Some(&occupied_pubkeys),
                owned_pubkey: existing_identity
                    .as_ref()
                    .map(|i| i.pubkey.as_str())
                    .or(preferred_pubkey.as_deref()),
            },
        )?
    };

    let identity = crate::state::Identity {
        pubkey: signer.pubkey.clone(),
        base_pubkey: base_pubkey.to_string(),
        agent_slug: agent_slug.to_string(),
        ordinal: signer.ordinal,
        session_id: session_id.to_string(),
        channel_h: h.to_string(),
        native_id: native_id.to_string(),
        alive: true,
        created_at: now_secs(),
    };
    if let Err(e) = state.with_store(|s| s.upsert_identity(&identity)) {
        state.release_session_signer(session_id);
        return Err(e);
    }
    Ok(signer)
}
