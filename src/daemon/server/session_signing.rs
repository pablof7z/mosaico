use super::*;

impl DaemonState {
    /// Resolve the signer selected at session start. Durable agents use their
    /// persisted config key; normal sessions derive from the management root.
    pub(in crate::daemon) fn session_signing_keys(&self, session_id: &str) -> Result<Keys> {
        if let Some(keys) = self.session_keys.lock().unwrap().get(session_id).cloned() {
            return Ok(keys);
        }
        let durable = self.with_store(|s| s.is_durable_agent_session(session_id))?;
        if durable {
            let session = self
                .with_store(|s| s.get_session(session_id))?
                .with_context(|| format!("durable session {session_id:?} is not registered"))?;
            let identity = crate::identity::load_or_create(
                &crate::config::edge_home(),
                &session.agent_slug,
                crate::util::now_secs(),
            )?;
            if identity.per_session_key || identity.pubkey_hex() != session.agent_pubkey {
                anyhow::bail!(
                    "durable signer configuration changed for agent {:?}",
                    session.agent_slug
                );
            }
            return Ok(identity.keys);
        }
        let mgmt = self.management_keys()?;
        Ok(crate::identity::derive_session_keys_v2(
            mgmt.secret_key(),
            session_id,
        ))
    }
}

/// A freshly minted per-session identity: the session's own signing keys plus
/// its read-side projection (pubkey, agent slug, session id).
pub(in crate::daemon::server) struct MintedSession {
    pub keys: Keys,
    pub identity: crate::identity::SessionIdentity,
    pub reclaimed_pubkey: Option<String>,
    pub durable_claim_acquired: bool,
}

pub(in crate::daemon::server) fn validate_live_session_identity(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    agent: &crate::identity::AgentIdentity,
) -> Result<()> {
    let durable = !agent.per_session_key;
    let expected = if durable {
        agent.pubkey_hex()
    } else {
        let mgmt = state.management_keys()?;
        crate::identity::derive_session_keys_v2(mgmt.secret_key(), &session.session_id)
            .public_key()
            .to_hex()
    };
    let stored_durable = state.with_store(|s| s.is_durable_agent_session(&session.session_id))?;
    if stored_durable != durable || session.agent_pubkey != expected {
        anyhow::bail!(
            "agent {:?} identity configuration changed while session {} is live; \
             end the live session before changing perSessionKey or its persisted key",
            session.agent_slug,
            session.session_id
        );
    }
    Ok(())
}

/// Select this session's signing identity.
///
/// Normal agents derive a unique resumable session key and lease a handle.
/// Agents configured with `perSessionKey:false` use their persisted config key,
/// claim the backend-wide durable-agent slot, and publish under the bare slug.
///
/// Records the selected pubkey in `identities`. Per-session identities retain
/// their native resume id; durable identities intentionally leave it empty.
pub(in crate::daemon::server) fn mint_session_identity(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent: &crate::identity::AgentIdentity,
    h: &str,
    native_id: &str,
    durable_reservation: Option<&str>,
) -> Result<MintedSession> {
    let agent_slug = agent.slug.as_str();
    let durable_agent = !agent.per_session_key;
    let keys = if durable_agent {
        agent.keys.clone()
    } else {
        let mgmt = state.management_keys()?;
        crate::identity::derive_session_keys_v2(mgmt.secret_key(), session_id)
    };
    let pubkey = keys.public_key().to_hex();
    let (codename, reclaimed_pubkey, durable_claim_acquired) = if durable_agent {
        let acquired = state.with_store(|s| {
            s.claim_durable_agent_session_with_reservation(
                &pubkey,
                agent_slug,
                session_id,
                durable_reservation,
                now_secs(),
            )
        })?;
        (String::new(), None, acquired)
    } else {
        let allocation =
            state.with_store(|s| s.allocate_handle(&pubkey, agent_slug, now_secs()))?;
        (allocation.codename, allocation.reclaimed_pubkey, false)
    };
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
        native_id: if durable_agent {
            String::new()
        } else {
            native_id.to_string()
        },
        alive: true,
        created_at: now_secs(),
    };
    if let Err(e) = state.with_store(|s| s.upsert_identity(&identity)) {
        state.release_session_signer(session_id);
        if durable_agent {
            state
                .with_store(|s| s.release_durable_agent_session(session_id))
                .ok();
        }
        return Err(e);
    }
    let identity = if durable_agent {
        crate::identity::SessionIdentity::durable_agent(
            pubkey,
            agent_slug.to_string(),
            session_id.to_string(),
        )
    } else {
        crate::identity::SessionIdentity::new(
            pubkey,
            agent_slug.to_string(),
            session_id.to_string(),
            codename,
        )
    };
    Ok(MintedSession {
        keys,
        identity,
        reclaimed_pubkey,
        durable_claim_acquired,
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
