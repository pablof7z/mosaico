use super::super::*;

// ── project_list ─────────────────────────────────────────────────────────────

/// List NIP-29 groups: refresh the local cache via the provider (which fetches
/// kind:39000 from the relay), then return the read-model list.
pub async fn rpc_project_list(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    // Provider fetches kind:39000 from the relay and upserts relay_channels.
    // Best-effort: a relay timeout must not prevent returning cached results.
    state.provider.refresh_project_list().await.ok();

    let projects: Vec<serde_json::Value> = state
        .with_store(|s| s.list_projects_read_model())
        .unwrap_or_default()
        .into_iter()
        .map(|c| serde_json::json!({ "slug": c.channel_h, "about": c.about }))
        .collect();

    Ok(serde_json::json!({ "projects": projects }))
}

// ── project_edit ─────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9002 (edit-metadata) event signed by the human user's
/// nsec. The relay validates admin rights and updates its kind:39000 accordingly.
pub async fn rpc_project_edit(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        description: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_edit params")?;

    let user_keys = state.management_keys()?;

    // NIP-29 edit-metadata: the wire shape lives in the nip29 lifecycle module.
    // The relay validates admin rights and re-publishes kind:39000.
    let builder = crate::fabric::nip29::lifecycle::group_edit_metadata(&p.project, &p.description)?;
    let event_id = state.transport.publish_signed(builder, &user_keys).await?;

    let confirmed = wait_for_channel_about(state, &p.project, &p.description).await;

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "project": p.project,
        "confirmed": confirmed,
    }))
}

// ── project_members ──────────────────────────────────────────────────────────

/// Return the cached NIP-29 membership roster for a project. Before reading the
/// cache, try to refresh kind:39002 from the relay so interactive project edits
/// start from the relay's current roster rather than only local optimistic state.
pub async fn rpc_project_members(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_members params")?;
    refresh_project_members_cache(state, &p.project).await;

    let member_pubkeys = state
        .with_store(|s| s.list_channel_members(&p.project))
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.pubkey)
        .collect::<Vec<_>>();
    crate::profile::warm(state, &member_pubkeys).await;

    let members = state
        .with_store(|s| s.list_channel_members(&p.project))
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let slug = state
                .with_store(|s| s.resolve_slug_for_pubkey(&m.pubkey).ok().flatten())
                .unwrap_or_default();
            serde_json::json!({ "pubkey": m.pubkey, "slug": slug, "role": m.role })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "project": p.project,
        "members": members,
    }))
}

async fn wait_for_channel_about(
    state: &Arc<DaemonState>,
    project: &str,
    description: &str,
) -> bool {
    for _ in 0..20 {
        state.provider.refresh_project_list().await.ok();
        let matches = state.with_store(|s| {
            s.channel_meta_read_model(project)
                .ok()
                .flatten()
                .map(|c| c.about)
                .as_deref()
                == Some(description)
        });
        if matches {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// Publish a NIP-29 kind:9000 (put-user) event to add a pubkey to the group.
/// Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
pub async fn rpc_project_add(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_add params")?;

    let pubkey_hex = resolve_project_member_pubkey_hex(&p.pubkey).await?;
    log_nip29_role_decision(
        &p.project,
        &pubkey_hex,
        "member",
        "rpc_project_add manual add via confirmed provider mutation",
    );

    let outcome = state
        .provider
        .grant_member_confirmed(&p.project, &pubkey_hex)
        .await;
    if outcome.is_rejected() {
        anyhow::bail!(
            "project_add rejected for {} in {}",
            crate::util::pubkey_short(&pubkey_hex),
            p.project
        );
    }

    Ok(serde_json::json!({
        "project": p.project,
        "pubkey": pubkey_hex,
        "confirmed": outcome.is_confirmed(),
    }))
}

// ── project_remove ───────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9001 (remove-user) event to remove a pubkey from the
/// group. Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
pub async fn rpc_project_remove(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_remove params")?;

    let pubkey_hex = resolve_pubkey_hex(&p.pubkey).await?;

    let outcome = state
        .provider
        .remove_member_confirmed(&p.project, &pubkey_hex)
        .await;
    if outcome.is_rejected() {
        anyhow::bail!(
            "project_remove rejected for {} in {}",
            crate::util::pubkey_short(&pubkey_hex),
            p.project
        );
    }

    Ok(serde_json::json!({
        "project": p.project,
        "pubkey": pubkey_hex,
        "confirmed": outcome.is_confirmed(),
    }))
}
