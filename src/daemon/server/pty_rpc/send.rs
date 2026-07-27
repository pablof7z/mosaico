use super::*;

#[derive(serde::Deserialize)]
struct PtySendParams {
    session: String,
}

pub(in crate::daemon::server) async fn rpc_pty_send(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: PtySendParams =
        serde_json::from_value(params.clone()).context("parsing pty_send params")?;

    let rec = state
        .with_store(|store| super::super::resolution::resolve_public_session(store, &p.session))?
        .with_context(|| "PTY send requires an npub, hex pubkey, or current handle")?;

    let Some(pty_id) = pty_session_for(state, &rec) else {
        return Ok(serde_json::json!({
            "injected": false,
            "reason": "no PTY endpoint registered for this session"
        }));
    };
    if !crate::pty::is_live(&pty_id) {
        return Ok(serde_json::json!({
            "injected": false,
            "pty_id": pty_id,
            "reason": "PTY endpoint probe failed; bounded lifecycle reconciliation will verify ownership"
        }));
    }

    let injected = crate::session_host::inject_pending_messages_pty(state, &rec, &pty_id).await?;
    if injected {
        Ok(serde_json::json!({ "injected": true, "pty_id": pty_id }))
    } else {
        Ok(serde_json::json!({
            "injected": false,
            "pty_id": pty_id,
            "reason": "no unread messages for this session"
        }))
    }
}

/// Call `ensure_channel_ready` for the launch scope (the channel if given, else
/// the root channel) before the hosted process is opened.
///
/// If the same agent slug already has a live session in the scope, logs a note
/// about the concurrent launch. The actual signer pubkey is selected and
/// admitted by `session_start`; pre-provisioning with the derivation root pubkey
/// would make the first session look like a duplicate to the ordinal allocator.
pub(in crate::daemon::server) async fn provision_before_spawn(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    channel: Option<&str>,
) -> Result<()> {
    let scope = channel.filter(|g| !g.is_empty()).unwrap_or(root);
    if scope.is_empty() {
        tracing::info!(slug, "provision: launching without a workspace channel");
        return Ok(());
    }
    let already_live = state
        .with_store(|s| s.list_running_sessions())
        .unwrap_or_default()
        .iter()
        .any(|r| {
            r.agent_slug == slug
                && state
                    .with_store(|store| store.has_session_route(&r.pubkey, scope).unwrap_or(false))
        });
    if already_live {
        tracing::info!(
            slug,
            scope,
            "provision: launching concurrent instance (agent already has live session)"
        );
    }

    let expect_member = state.backend_pubkey().unwrap_or_default();
    let already_ready = state.with_store(|store| {
        store.get_channel(scope).ok().flatten().is_some()
            && store
                .is_channel_admin(scope, &expect_member)
                .unwrap_or(false)
    });
    if already_ready {
        tracing::debug!(
            slug,
            channel = scope,
            "provision: using relay-confirmed cached channel readiness"
        );
        return Ok(());
    }

    let timeout = std::time::Duration::from_secs(20);
    let parent_hint = channel
        .filter(|g| !g.is_empty() && *g != root)
        .map(|_| root);
    let channel_name = state
        .with_store(|s| s.get_channel(scope))
        .ok()
        .flatten()
        .map(|c| c.name)
        .unwrap_or_default();
    tracing::info!(
        slug,
        channel = scope,
        channel_name,
        "provision: ensuring channel ready"
    );
    let ctx = crate::fabric::nip29::readiness::ChannelCtx {
        channel: scope,
        expect_member: &expect_member,
        parent_hint,
        name: None,
        repair_whitelisted_admins: true,
    };
    match tokio::time::timeout(timeout, state.provider.ensure_channel_ready(ctx)).await {
        Ok(crate::fabric::nip29::readiness::ChannelGate::Degraded(error)) => tracing::warn!(
            slug,
            channel = scope,
            error = %error,
            "provision: channel readiness degraded before spawn; opening local session anyway"
        ),
        Ok(_) => {}
        Err(_) => tracing::warn!(
            slug,
            channel = scope,
            "provision: channel readiness timed out before spawn; opening local session anyway"
        ),
    }
    Ok(())
}

// ── pty_attach ────────────────────────────────────────────────────────────────
