use super::*;

mod keys;

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
    let stored_durable = state.with_store(|s| s.is_durable_agent_session(&session.session_id))?;
    if stored_durable != durable {
        anyhow::bail!(
            "agent {:?} identity configuration changed while session {} is live; \
             end the live session before changing perSessionKey or its persisted key",
            session.agent_slug,
            session.session_id
        );
    }
    let expected = if durable {
        agent.pubkey_hex()
    } else {
        let signer_salt = state
            .with_store(|s| s.session_signer_salt(&session.agent_pubkey))?
            .with_context(|| format!("pubkey {:?} has no signer material", session.agent_pubkey))?;
        let mgmt = state.management_keys()?;
        crate::identity::derive_session_keys(mgmt.secret_key(), &signer_salt)?
            .public_key()
            .to_hex()
    };
    if session.agent_pubkey != expected {
        anyhow::bail!(
            "agent {:?} identity configuration changed while session {} is live; \
             end the live session before changing perSessionKey or its persisted key",
            session.agent_slug,
            session.session_id
        );
    }
    Ok(())
}

pub(in crate::daemon::server) fn validate_agent_identity_admission(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent: &crate::identity::AgentIdentity,
) -> Result<()> {
    let desired_durable = !agent.per_session_key;
    let conflicts = state.with_store(|s| {
        Ok::<_, anyhow::Error>(s.list_alive_sessions()?.into_iter().find(|session| {
            session.agent_slug == agent.slug
                && session.session_id != session_id
                && (desired_durable
                    || s.is_durable_agent_session(&session.session_id)
                        .unwrap_or(false))
        }))
    })?;
    if let Some(existing) = conflicts {
        anyhow::bail!(
            "agent {:?} has live session {} under an incompatible identity mode; \
             end or attach/add the live session before launching another",
            agent.slug,
            existing.session_id
        );
    }
    Ok(())
}

pub(in crate::daemon::server) fn validate_launch_reservation(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    reservation: Option<&str>,
) -> Result<()> {
    if agent.per_session_key && reservation.is_some() {
        state
            .with_store(|s| s.release_durable_agent_session(reservation.unwrap()))
            .ok();
        anyhow::bail!(
            "agent {:?} identity mode changed after durable launch reservation; retry launch",
            agent.slug
        );
    }
    Ok(())
}

/// Native-host identity data plus an optional operator-selected public name.
pub(in crate::daemon::server) struct SessionIdentityInput<'a> {
    native_id: &'a str,
    session_name: Option<&'a str>,
}

impl<'a> SessionIdentityInput<'a> {
    pub(in crate::daemon::server) fn new(
        native_id: &'a str,
        session_name: Option<&'a str>,
    ) -> Self {
        Self {
            native_id,
            session_name,
        }
    }
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
    input: SessionIdentityInput<'_>,
    durable_reservation: Option<&str>,
) -> Result<MintedSession> {
    let agent_slug = agent.slug.as_str();
    let durable_agent = !agent.per_session_key;
    let now = now_secs();
    let (keys, pubkey, codename, reclaimed_pubkey, durable_claim_acquired) = if durable_agent {
        let keys = agent.keys.clone();
        let pubkey = keys.public_key().to_hex();
        let claim = state.with_store(|s| {
            s.claim_durable_agent_session_with_reservation(
                &pubkey,
                agent_slug,
                session_id,
                durable_reservation,
                now,
            )
        });
        let acquired = match claim {
            Ok(acquired) => acquired,
            Err(error) => {
                if let Some(reservation) = durable_reservation {
                    state
                        .with_store(|s| s.release_durable_agent_session(reservation))
                        .ok();
                }
                return Err(error);
            }
        };
        let record = crate::state::Identity {
            pubkey: pubkey.clone(),
            agent_slug: agent_slug.to_string(),
            codename: String::new(),
            session_id: session_id.to_string(),
            channel_h: h.to_string(),
            native_id: String::new(),
            alive: true,
            created_at: now,
        };
        if let Err(error) = state.with_store(|s| s.upsert_identity(&record)) {
            if acquired {
                state
                    .with_store(|s| s.release_durable_agent_session(session_id))
                    .ok();
            }
            return Err(error);
        }
        (keys, pubkey, String::new(), None, acquired)
    } else {
        let mgmt = state.management_keys()?;
        state.with_store(|store| {
            let prior = store.identity_for_session(session_id)?;
            let signer_salt = if let Some(identity) = prior.as_ref() {
                store
                    .session_signer_salt(&identity.pubkey)?
                    .with_context(|| {
                        format!("pubkey {:?} has no signer material", identity.pubkey)
                    })?
            } else {
                crate::identity::new_session_signer_salt()
            };
            let keys = crate::identity::derive_session_keys(mgmt.secret_key(), &signer_salt)?;
            let pubkey = keys.public_key().to_hex();
            if prior
                .as_ref()
                .is_some_and(|identity| identity.pubkey != pubkey)
            {
                anyhow::bail!("stored signer material does not reproduce session pubkey");
            }
            store.bind_session_signer(&pubkey, &signer_salt)?;
            let allocation = match input.session_name {
                Some(name) => store.allocate_custom_handle(&pubkey, agent_slug, name, now)?,
                None => store.allocate_handle(&pubkey, agent_slug, now)?,
            };
            let record = crate::state::Identity {
                pubkey: pubkey.clone(),
                agent_slug: agent_slug.to_string(),
                codename: allocation.codename.clone(),
                session_id: session_id.to_string(),
                channel_h: h.to_string(),
                native_id: input.native_id.to_string(),
                alive: true,
                created_at: now,
            };
            store.upsert_identity(&record)?;
            Ok::<_, anyhow::Error>((
                keys,
                pubkey,
                allocation.codename,
                allocation.reclaimed_pubkey,
                false,
            ))
        })?
    };
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
    let keys = state.session_signing_keys(pubkey)?;
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

#[cfg(test)]
#[path = "session_signing/tests.rs"]
mod tests;
