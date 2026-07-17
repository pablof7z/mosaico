use super::*;

mod keys;

/// Identity selected before a local runtime starts. The pubkey is authoritative;
/// the handle is its sole public alias.
pub(crate) struct PreparedIdentity {
    pub(crate) keys: Keys,
    pub(crate) identity: crate::identity::SessionIdentity,
    pub(crate) reclaimed_pubkey: Option<String>,
}

pub(crate) fn validate_live_session_identity(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    agent: &crate::identity::AgentIdentity,
) -> Result<()> {
    let derived = state.with_store(|store| store.is_derived_session_pubkey(&session.pubkey))?;
    if derived != agent.per_session_key {
        anyhow::bail!(
            "agent {:?} identity configuration changed while pubkey {} is live; \
             end the live runtime before changing perSessionKey",
            session.agent_slug,
            session.pubkey
        );
    }
    let expected = if derived {
        let signer_salt = state
            .with_store(|store| store.session_signer_salt(&session.pubkey))?
            .with_context(|| format!("pubkey {:?} has no signer material", session.pubkey))?;
        let mgmt = state.management_keys()?;
        crate::identity::derive_session_keys(mgmt.secret_key(), &signer_salt)?
            .public_key()
            .to_hex()
    } else {
        agent
            .pubkey_hex()
            .context("durable agent has no configured key")?
    };
    if session.pubkey != expected {
        anyhow::bail!(
            "agent {:?} signing configuration no longer reproduces pubkey {}",
            session.agent_slug,
            session.pubkey
        );
    }
    Ok(())
}

/// Reject an agent identity-mode change while an incompatible runtime for the
/// same configured agent is still active. Ordinary per-session identities may
/// otherwise run concurrently because each owns a distinct pubkey.
pub(in crate::daemon::server) fn validate_agent_identity_admission(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
) -> Result<()> {
    let conflict = state.with_store(|store| {
        for session in store
            .list_alive_sessions()?
            .into_iter()
            .filter(|session| session.agent_slug == agent.slug)
        {
            let derived = store.is_derived_session_pubkey(&session.pubkey)?;
            if !agent.per_session_key || !derived {
                return Ok::<_, anyhow::Error>(Some(session));
            }
        }
        Ok(None)
    })?;
    if let Some(existing) = conflict {
        anyhow::bail!(
            "agent {:?} has an active runtime under pubkey {}; end or attach to it \
             before changing identity mode",
            agent.slug,
            existing.pubkey
        );
    }
    Ok(())
}

/// Select signing keys and the single public handle before process spawn.
pub(crate) fn prepare_session_identity(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    session_name: Option<&str>,
) -> Result<PreparedIdentity> {
    validate_agent_identity_admission(state, agent)?;
    let now = now_secs();
    if !agent.per_session_key {
        if session_name.is_some() {
            anyhow::bail!(
                "durable agent {:?} already uses its configured public handle {:?}",
                agent.slug,
                agent.slug
            );
        }
        let pubkey = agent
            .pubkey_hex()
            .context("durable agent has no configured key")?;
        if state.with_store(|store| store.is_derived_session_pubkey(&pubkey))? {
            anyhow::bail!("durable pubkey {pubkey} already has derived signer material");
        }
        return Ok(PreparedIdentity {
            keys: agent
                .keys
                .clone()
                .context("durable agent has no configured key")?,
            identity: crate::identity::SessionIdentity::durable_agent(pubkey, agent.slug.clone()),
            reclaimed_pubkey: None,
        });
    }

    let mgmt = state.management_keys()?;
    let (keys, pubkey, allocation) = state.with_store(|store| {
        store.reserve_derived_identity(&agent.slug, session_name, now, |signer_salt| {
            let keys = crate::identity::derive_session_keys(mgmt.secret_key(), signer_salt)?;
            let pubkey = keys.public_key().to_hex();
            Ok((keys, pubkey))
        })
    })?;
    Ok(PreparedIdentity {
        keys,
        identity: crate::identity::SessionIdentity::new(
            pubkey,
            agent.slug.clone(),
            allocation.handle,
            false,
        ),
        reclaimed_pubkey: allocation.reclaimed_pubkey,
    })
}

/// Reconstruct a previously prepared identity by its authoritative pubkey.
pub(crate) fn load_session_identity(
    state: &Arc<DaemonState>,
    pubkey: &str,
    agent: &crate::identity::AgentIdentity,
) -> Result<PreparedIdentity> {
    let session = state
        .with_store(|store| store.get_session(pubkey))?
        .with_context(|| format!("unknown local session pubkey {pubkey}"))?;
    validate_live_session_identity(state, &session, agent)?;
    let identity = state
        .with_store(|store| store.session_identity(pubkey))?
        .with_context(|| format!("pubkey {pubkey} has no public handle projection"))?;
    Ok(PreparedIdentity {
        keys: state.session_signing_keys(pubkey)?,
        identity,
        reclaimed_pubkey: None,
    })
}

pub(in crate::daemon::server) async fn retire_reclaimed_profile(
    state: &Arc<DaemonState>,
    reclaimed_pubkey: Option<&str>,
) -> Result<()> {
    let Some(pubkey) = reclaimed_pubkey else {
        return Ok(());
    };
    let Some(session) = state.with_store(|store| store.get_session(pubkey))? else {
        tracing::warn!(pubkey, "reclaimed handle had no local runtime projection");
        return Ok(());
    };
    let keys = state.session_signing_keys(pubkey)?;
    let npub = crate::idref::npub(pubkey).unwrap_or_else(|| pubkey.to_string());
    let profile = crate::domain::Profile::agent(
        crate::domain::AgentRef::new(pubkey.to_string(), npub.clone()),
        session.agent_slug.clone(),
        state.host.clone(),
        state.owners.clone(),
    );
    let domain = crate::domain::DomainEvent::Profile(profile);
    state.with_store(|store| {
        store.upsert_profile_with_agent_slug(
            pubkey,
            &npub,
            &npub,
            &session.agent_slug,
            &state.host,
            false,
            now_secs(),
        )?;
        Ok::<_, anyhow::Error>(())
    })?;
    state
        .provider
        .enqueue(&domain, &keys)
        .await
        .with_context(|| format!("queueing reclaimed handle retirement profile for {pubkey}"))?;
    Ok(())
}

#[cfg(test)]
#[path = "session_signing/tests.rs"]
mod tests;
