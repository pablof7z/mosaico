//! First-class local sessions for authenticated remote MCP callers.

use super::*;

#[derive(serde::Deserialize)]
struct Params {
    actor_key: String,
    actor_kind: String,
    channel: String,
}

pub(super) async fn rpc_resolve(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: Params = serde_json::from_value(params.clone()).context("mcp_actor_resolve params")?;
    anyhow::ensure!(
        matches!(p.actor_kind.as_str(), "openai" | "grok"),
        "unsupported MCP actor kind"
    );
    anyhow::ensure!(
        p.actor_key.len() >= 32
            && p.actor_key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid redacted MCP actor key"
    );
    anyhow::ensure!(
        !p.channel.trim().is_empty(),
        "MCP actor channel is required"
    );

    let now = now_secs();
    let (pubkey, prepared) = {
        let _actor_lane = state.mcp_actor_sync.lock().await;
        if let Some(pubkey) = state.with_store(|s| s.mcp_actor_pubkey(&p.actor_key))? {
            state.with_store(|s| s.activate_mcp_actor(&p.actor_key, &pubkey, now))?;
            (pubkey, None)
        } else {
            let agent_slug = format!("mcp-{}", p.actor_kind);
            let agent = crate::identity::AgentIdentity::per_session(&agent_slug, "mcp");
            let prepared = prepare_session_identity(state, &agent, None)?;
            let pubkey = prepared.identity.pubkey.clone();
            state.with_store(|s| {
                s.reserve_mcp_actor_session(&pubkey, &agent_slug, &p.channel, now)?;
                s.bind_mcp_actor(&p.actor_key, &p.actor_kind, &pubkey, now)
            })?;
            (pubkey, Some(prepared))
        }
    };

    let session = state
        .with_store(|s| s.get_session(&pubkey))?
        .context("MCP actor session disappeared")?;
    super::channel_membership_rpc::ensure_joinable(state, &session, &p.channel).await?;
    if let Some(prepared) = prepared {
        publish_profile(state, &prepared, &p.channel).await?;
    }
    Ok(serde_json::json!({ "pubkey": pubkey }))
}

pub(super) async fn ensure_membership_if_actor(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    channel: &str,
) -> Result<()> {
    if !state.with_store(|store| store.is_mcp_actor_pubkey(&session.pubkey))? {
        return Ok(());
    }
    super::channel_membership_rpc::ensure_joinable(state, session, channel).await?;
    Ok(())
}

async fn publish_profile(
    state: &Arc<DaemonState>,
    prepared: &PreparedIdentity,
    workspace: &str,
) -> Result<()> {
    let profile = crate::domain::Profile::agent(
        prepared.identity.agent_ref(),
        prepared.identity.slug.clone(),
        state.host.clone(),
        state.owners.clone(),
    )
    .with_workspace(workspace.to_string());
    state
        .provider
        .enqueue(
            &crate::domain::DomainEvent::Profile(profile),
            &prepared.keys,
        )
        .await?;
    let npub = crate::idref::npub(&prepared.identity.pubkey)
        .unwrap_or_else(|| prepared.identity.pubkey.clone());
    state.with_store(|s| {
        s.upsert_profile_with_agent_slug(
            &prepared.identity.pubkey,
            &prepared.identity.handle,
            &npub,
            &prepared.identity.slug,
            &state.host,
            false,
            now_secs(),
        )?;
        Ok::<_, anyhow::Error>(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repeated_actor_resolution_reuses_the_exact_session() {
        let state = DaemonState::new_for_test().await;
        let agent = crate::identity::AgentIdentity::per_session("mcp-openai", "mcp");
        let prepared = prepare_session_identity(&state, &agent, None).unwrap();
        let pubkey = prepared.identity.pubkey.clone();
        let actor_key = "mcp1_redacted_actor_key_1234567890";
        state.with_store(|store| {
            store
                .reserve_mcp_actor_session(&pubkey, "mcp-openai", "mosaico", 1)
                .unwrap();
            store
                .bind_mcp_actor(actor_key, "openai", &pubkey, 1)
                .unwrap();
            store
                .upsert_channel_member("mosaico", &pubkey, "member", 1)
                .unwrap();
        });

        let value = rpc_resolve(
            &state,
            &serde_json::json!({
                "actor_key": actor_key,
                "actor_kind": "openai",
                "channel": "mosaico",
            }),
        )
        .await
        .unwrap();
        assert_eq!(value["pubkey"], pubkey);
    }
}
