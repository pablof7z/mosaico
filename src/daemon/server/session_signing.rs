use super::*;

/// Select the durable ordinal identity for a session (issue #47).
///
/// `base_keys`/`base_pubkey` are the agent's durable ordinal-0 identity. The
/// allocator picks ordinal 0 (sign with the base key) for the first live session
/// of the agent, and the next free durable ordinal (`smith1`, …) for concurrent
/// ones across every channel. A session's already-bound ordinal (same-process
/// reassert or cross-restart revive) is honored so its identity is stable.
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
    let preferred = hint_ordinal.or_else(|| existing_identity.as_ref().map(|i| i.ordinal));
    let preferred_pubkey = hint_ordinal.map(|ordinal| {
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
        occupied.extend(
            s.identities_for_base(base_pubkey)
                .unwrap_or_default()
                .into_iter()
                .filter(|i| i.alive)
                .map(|i| i.pubkey),
        );
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

/// Admit an ordinal (>0) signer to the channel as a NIP-29 member before routing
/// use. Membership is materialized from the relay's 39002 reflection, so this
/// only performs the relay-side add; the local `relay_channel_members` cache
/// updates when the reflected 39002 lands.
pub(in crate::daemon::server) async fn admit_ordinal_signer(
    state: &Arc<DaemonState>,
    project: &str,
    session_pubkey: &str,
) -> Result<()> {
    let add = state.provider.nip29_add_member(project, session_pubkey);
    let accepted = tokio::time::timeout(std::time::Duration::from_secs(8), add)
        .await
        .unwrap_or(false);
    if !accepted {
        anyhow::bail!(
            "NIP-29 admission failed for ordinal signer {} in {project}",
            pubkey_short(session_pubkey)
        );
    }
    Ok(())
}
