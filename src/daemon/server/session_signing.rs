use super::*;

/// Select the durable ordinal identity for a session in room `h` (issue #47).
///
/// `base_keys`/`base_pubkey` are the agent's durable ordinal-0 identity. The
/// allocator picks ordinal 0 (sign with the base key) for the first session of
/// the agent in the room, and the next free durable ordinal (`smith1`, …) for
/// concurrent ones. A session's already-bound ordinal (same-process reassert or
/// cross-restart revive) is honored so its identity is stable.
///
/// Persists the durable inventory (`agent_ordinals`) and the `(pubkey, h)` route
/// that binds the ordinal to this live session and its native harness id (the
/// resume key). Dual-writes the legacy `session_pubkeys` table so existing
/// routing keeps working until the Phase 3 cutover to `(pubkey, h)`.
#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn select_session_signer(
    state: &Arc<DaemonState>,
    session_id: &str,
    base_keys: &Keys,
    base_pubkey: &str,
    agent_slug: &str,
    h: &str,
    harness_kind: &str,
    native_id: &str,
    hint_ordinal: Option<u32>,
) -> Result<session_signer::SelectedSigner> {
    // Honor (in priority order): an explicit spawn hint (mention-driven exact
    // ordinal), then a session's already-bound ordinal (reassert / restart), so
    // its durable identity survives.
    let preferred = hint_ordinal.or_else(|| {
        state
            .with_store(|s| s.identity_route_for_session(session_id))
            .map(|r| r.ordinal)
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
            },
        )?
    };

    let route = crate::state::IdentityRoute {
        pubkey: signer.pubkey.clone(),
        h: h.to_string(),
        session_id: session_id.to_string(),
        base_pubkey: base_pubkey.to_string(),
        agent_slug: agent_slug.to_string(),
        ordinal: signer.ordinal,
        label: signer.label.clone(),
        harness_kind: harness_kind.to_string(),
        native_id: native_id.to_string(),
        alive: true,
    };
    if let Err(e) = state.with_store(|s| {
        if signer.ordinal > 0 {
            s.ensure_agent_ordinal(
                base_pubkey,
                agent_slug,
                signer.ordinal,
                &signer.pubkey,
                now_secs(),
            )?;
            // Dual-write legacy table (ordinal 0 == base agent never had a row).
            s.upsert_session_pubkey(
                &signer.pubkey,
                session_id,
                base_pubkey,
                agent_slug,
                now_secs(),
            )?;
        }
        s.upsert_identity_route(&route, now_secs())?;
        Ok::<(), anyhow::Error>(())
    }) {
        state.release_session_signer(session_id);
        return Err(e);
    }
    Ok(signer)
}

pub(in crate::daemon::server) async fn admit_transient_signer(
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
            "NIP-29 admission failed for transient signer {} in {project}",
            pubkey_short(session_pubkey)
        );
    }
    state.with_store(|s| s.upsert_group_member(project, session_pubkey, "member", now_secs()))?;
    Ok(())
}
