use super::*;

/// A freshly minted per-session identity: the session's own signing keys plus
/// its read-side projection (pubkey, agent slug, session id).
pub(in crate::daemon::server) struct MintedSession {
    pub keys: Keys,
    pub identity: crate::identity::SessionIdentity,
    pub reclaimed_pubkey: Option<String>,
}

/// Mint (or deterministically re-derive) this session's OWN keypair.
///
/// `nsec = derive_session_keys_v2(management_secret, session_id)`. The management
/// key is the per-machine root; a resumed session (same `session_id`) re-derives
/// the identical pubkey. Per-session keys are unique by construction, so there is
/// no occupancy/ordinal/collision logic — every session simply gets its own key.
///
/// Records the minted pubkey into the append-only `identities` cache, binding it
/// to this live session + its harness-native id (the resume key), so a later
/// `#p`-tagged mention resolves back to the right session.
pub(in crate::daemon::server) fn mint_session_identity(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent_slug: &str,
    h: &str,
    native_id: &str,
) -> Result<MintedSession> {
    let mgmt = state.management_keys()?;
    let keys = crate::identity::derive_session_keys_v2(mgmt.secret_key(), session_id);
    let pubkey = keys.public_key().to_hex();
    let allocation = state.with_store(|s| s.allocate_handle(&pubkey, agent_slug, now_secs()))?;
    let codename = allocation.codename;
    state
        .session_keys
        .lock()
        .unwrap()
        .insert(session_id.to_string(), keys.clone());

    let identity = crate::state::Identity {
        pubkey: pubkey.clone(),
        agent_slug: agent_slug.to_string(),
        codename: codename.clone(),
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
    Ok(MintedSession {
        keys,
        identity: crate::identity::SessionIdentity::new(
            pubkey,
            agent_slug.to_string(),
            session_id.to_string(),
            codename,
        ),
        reclaimed_pubkey: allocation.reclaimed_pubkey,
    })
}

pub(in crate::daemon::server) async fn retire_reclaimed_profile(
    state: &Arc<DaemonState>,
    reclaimed_pubkey: Option<&str>,
) -> Result<()> {
    let Some(pubkey) = reclaimed_pubkey else {
        return Ok(());
    };
    let Some(identity) = state.with_store(|s| s.get_identity(pubkey))? else {
        tracing::warn!(
            pubkey,
            "reclaimed orphan handle had no profile identity to retire"
        );
        return Ok(());
    };
    let keys = state.session_signing_keys(&identity.session_id)?;
    let npub = crate::idref::npub(pubkey).unwrap_or_else(|| pubkey.to_string());
    let agent_slug = identity.agent_slug;
    let profile = crate::domain::Profile::agent(
        crate::domain::AgentRef::new(pubkey.to_string(), npub.clone()),
        agent_slug.clone(),
        state.host.clone(),
        state.owners.clone(),
    );
    let domain = crate::domain::DomainEvent::Profile(profile);
    let event = state.provider.encode(&domain)?.sign_with_keys(&keys)?;
    let event_json = serde_json::to_string(&event)?;
    state.with_store(|s| {
        s.upsert_profile_with_agent_slug(
            pubkey,
            &npub,
            &npub,
            &agent_slug,
            &state.host,
            false,
            now_secs(),
        )?;
        s.enqueue_outbox(&event_json, now_secs())?;
        Ok::<_, anyhow::Error>(())
    })?;
    state.outbox_notify.notify_waiters();
    if let Err(error) = state.provider.publish(&domain, &keys).await {
        tracing::warn!(pubkey, %error, "reclaimed handle retirement profile queued for retry");
    }
    Ok(())
}
