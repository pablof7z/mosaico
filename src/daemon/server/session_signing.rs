use super::*;

/// A freshly minted per-session identity: the session's own signing keys plus
/// its read-side projection (pubkey, slug, codename).
pub(in crate::daemon::server) struct MintedSession {
    pub keys: Keys,
    pub identity: crate::identity::SessionIdentity,
}

/// Mint (or deterministically re-derive) this session's OWN keypair.
///
/// `nsec = derive_session_keys_v2(management_secret, session_id)`. The management
/// key is the per-machine root; a resumed session (same `session_id`) re-derives
/// the identical pubkey. Per-session keys are unique by construction, so there is
/// no occupancy/ordinal/collision logic — every session simply gets its own key.
///
/// Records the minted pubkey into the append-only `identities` cache, binding it
/// to this live session + its harness-native id (the resume key) and the
/// memorable codename it publishes under, so a later `#p`-tagged mention resolves
/// back to the right session.
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
    let codename = crate::util::friendly_short_code(session_id);
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
        identity: crate::identity::SessionIdentity::new(pubkey, agent_slug.to_string(), codename),
    })
}
