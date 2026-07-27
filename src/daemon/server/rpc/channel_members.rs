use super::super::*;

const CHANNEL_MEMBER_READY_TIMEOUT: Duration = Duration::from_secs(90);

// ── root_channels ────────────────────────────────────────────────────────────

/// List NIP-29 root channels: refresh the local cache via the provider (which
/// fetches kind:39000 from the relay), then return the read-model list.
pub async fn rpc_root_channels(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    // Provider fetches kind:39000 from the relay and upserts relay_channels.
    // Best-effort: a relay timeout must not prevent returning cached results.
    state.provider.refresh_root_channels().await.ok();

    let channels: Vec<serde_json::Value> = state
        .with_store(|s| s.list_root_channels())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            let channel =
                state.with_store(|s| crate::channel_ref::full_channel_ref(s, &c.channel_h));
            (!channel.is_empty())
                .then(|| serde_json::json!({ "channel": channel, "about": c.about }))
        })
        .collect();

    Ok(serde_json::json!({ "channels": channels }))
}

// ── channel_members ──────────────────────────────────────────────────────────

/// Return the cached NIP-29 membership roster for a channel. Before reading the
/// cache, refresh admin/member snapshots from the relay so interactive
/// membership edits start from relay state rather than only local optimistic
/// state.
pub async fn rpc_channel_members(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_members params")?;
    absolute::require_full_path("channel", &p.channel)?;
    let channel_h =
        match state.with_store(|s| absolute::resolve_absolute_channel_ref(s, &p.channel)) {
            ChannelResolution::Unique(channel_h) => channel_h,
            ChannelResolution::NotFound => {
                anyhow::bail!(
                    "{}",
                    state.with_store(|s| absolute::describe_missing_channel(s, &p.channel))
                )
            }
        };
    refresh_channel_members_cache(state, &channel_h).await;

    let member_pubkeys = state
        .with_store(|s| s.list_channel_members(&channel_h))
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.pubkey)
        .collect::<Vec<_>>();
    crate::profile::warm(state, &member_pubkeys).await;

    let members = state
        .with_store(|s| s.list_channel_members(&channel_h))
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
        "channel": p.channel,
        "members": members,
    }))
}

/// `channel add <pubkey|npub|nip05> <channel> [--admin]` — add a human member to
/// a channel. Resolves its full path globally to the internal group id, ensures
/// the channel is ready (management is admin), then publishes an explicit
/// NIP-29 kind:9000 put-user granting `member` (or `admin` when `admin`), read
/// back for confirmation.
pub async fn rpc_channel_add_member(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        pubkey: String,
        #[serde(default)]
        admin: bool,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_add_member params")?;

    let channel_h = match resolve_add_channel(state, params, &p.channel)? {
        ChannelResolution::Unique(h) => h,
        ChannelResolution::NotFound => {
            anyhow::bail!(
                "{}",
                state.with_store(|s| absolute::describe_missing_channel(s, &p.channel))
            )
        }
    };
    let channel = state.with_store(|s| crate::channel_ref::full_channel_ref(s, &channel_h));

    let pubkey_hex = resolve_channel_member_pubkey_hex(&p.pubkey).await?;
    let role = if p.admin { "admin" } else { "member" };
    log_nip29_role_decision(
        &channel_h,
        &pubkey_hex,
        role,
        "rpc_channel_add_member manual add via confirmed provider mutation",
    );

    let parent_hint = state
        .with_store(|s| s.channel_parent(&channel_h).unwrap_or(None))
        .filter(|parent| !parent.is_empty());
    let ready = state
        .provider
        .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
            channel: &channel_h,
            expect_member: &pubkey_hex,
            parent_hint: parent_hint.as_deref(),
            name: None,
            repair_whitelisted_admins: true,
        });
    let gate = tokio::time::timeout(CHANNEL_MEMBER_READY_TIMEOUT, ready)
        .await
        .with_context(|| {
            format!(
                "channel_add_member timed out verifying channel {} after {}s",
                channel,
                CHANNEL_MEMBER_READY_TIMEOUT.as_secs()
            )
        })?;
    gate.require_ready(format!(
        "channel_add_member could not verify channel {}",
        channel
    ))?;

    // Explicit grant of the requested role. `ensure_channel_ready` above only
    // guarantees the management key is admin; the member/admin put-user is
    // published here and read back for confirmation.
    let outcome = if p.admin {
        state
            .provider
            .grant_admin_confirmed(&channel_h, &pubkey_hex)
            .await
    } else {
        state
            .provider
            .grant_member_confirmed(&channel_h, &pubkey_hex)
            .await
    };
    outcome.require_confirmed(format!(
        "updating channel {} participant {}",
        channel,
        crate::util::pubkey_short(&pubkey_hex)
    ))?;

    Ok(serde_json::json!({
        "channel": channel,
        "pubkey": pubkey_hex,
        "role": role,
        "confirmed": true,
    }))
}

/// Resolve a full-path reference for `channel add`, globally —
/// not scoped to the caller's workspace. Still requires a resolvable caller
/// context (session anchor, else the invoking directory) as a precondition.
fn resolve_add_channel(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    reference: &str,
) -> Result<ChannelResolution> {
    let anchor = CallerAnchor::from_params(params);
    // Resolution below is global (not scoped to a caller workspace), but
    // channel add still requires a resolvable caller context.
    if resolve_session_inner(state, &anchor, ResolveScope::Strict).is_err() {
        let cwd = params
            .get("cwd")
            .and_then(serde_json::Value::as_str)
            .filter(|cwd| !cwd.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        crate::daemon::workspace_path::channel_for_path(&cwd)
            .context("channel add must run inside an agent session or workspace directory")?;
    }
    absolute::require_full_path("channel", reference)?;
    Ok(state.with_store(|s| absolute::resolve_absolute_channel_ref(s, reference)))
}

// ── channel_remove_member ─────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9001 (remove-user) event to remove a pubkey from the
/// group. Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
pub async fn rpc_channel_remove_member(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_remove_member params")?;

    let pubkey_hex = resolve_pubkey_hex(&p.pubkey).await?;
    absolute::require_full_path("channel", &p.channel)?;
    let channel_h =
        match state.with_store(|s| absolute::resolve_absolute_channel_ref(s, &p.channel)) {
            ChannelResolution::Unique(channel_h) => channel_h,
            ChannelResolution::NotFound => {
                anyhow::bail!(
                    "{}",
                    state.with_store(|s| absolute::describe_missing_channel(s, &p.channel))
                )
            }
        };

    let outcome = state
        .provider
        .remove_member_confirmed(&channel_h, &pubkey_hex)
        .await;
    let confirmed = outcome.is_confirmed();
    outcome.require_confirmed(format!(
        "removing channel {} participant {}",
        p.channel,
        crate::util::pubkey_short(&pubkey_hex)
    ))?;

    Ok(serde_json::json!({
        "channel": p.channel,
        "pubkey": pubkey_hex,
        "confirmed": confirmed,
    }))
}
