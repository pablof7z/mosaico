use super::*;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct WhoParams {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    all_projects: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, alias = "env_session")]
    harness_session: Option<String>,
    #[serde(default)]
    tmux_pane: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

/// `who`: build the snapshot with the SAME function the CLI used. The client
/// renders it with the existing renderers, so output is byte-identical. The
/// daemon resolves the current project the same way the old CLI did.
pub(in crate::daemon::server) fn rpc_who(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: WhoParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let anchor = CallerAnchor::from_params(params);
    let caller_rec = if p.all_projects {
        None
    } else if p.tmux_pane.as_deref().filter(|s| !s.is_empty()).is_some()
        || p.harness_session
            .as_deref()
            .filter(|s| !s.is_empty())
            .is_some()
        || p.watch_pid.is_some()
    {
        Some(resolve_session_inner(state, &anchor, ResolveScope::Strict)?)
    } else if p.agent.as_deref().filter(|s| !s.is_empty()).is_some()
        || p.group.as_deref().filter(|s| !s.is_empty()).is_some()
    {
        anyhow::bail!(
            "who needs an exact live session anchor; agent/channel env alone is not session context"
        );
    } else {
        None
    };
    let current_project = if p.all_projects {
        None
    } else if let Some(rec) = caller_rec.as_ref() {
        Some(rec.channel_h.clone())
    } else {
        Some(p.project.clone().unwrap_or_else(|| {
            let cwd = p
                .cwd
                .clone()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            crate::project::resolve(&cwd).unwrap_or_default()
        }))
    };
    let now = now_secs();
    let host = state.host.clone();
    let snapshot = state
        .with_store(|s| crate::cli::load_who_snapshot(s, current_project.as_deref(), now, &host))?;
    let mut out = serde_json::to_value(snapshot)?;

    // Attach the UNIFIED fabric view (same format as the hook injection — decision
    // A) whenever a single current channel resolves. `--all-projects` has no single
    // scope, so it keeps the cross-project snapshot table. The caller (this session,
    // when run inside an agent) is marked `(you)` and excluded from peer echoes.
    if let Some(scope) = current_project.as_deref() {
        // Reuse the exact caller session, when present, for both the fabric
        // `(you)` match and the folded-in `self` identity block (issue #99).
        // Deliberately no project-scan fallback: `who` must not masquerade as a
        // session just because some live sibling exists in the same repository.
        let rec = caller_rec.as_ref();
        // Issue #98: the caller's ONE authoritative agent-instance identity — the
        // selected pubkey + ordinal label every publisher signs with. Computed
        // OUTSIDE `with_store` because `session_instance` locks the store itself.
        let instance = rec.map(|rec| state.session_instance(rec));
        let (self_slug, self_pubkey) = instance
            .as_ref()
            .map(|i| (i.display_slug(), i.pubkey.clone()))
            .unwrap_or_default();
        let edge = crate::config::edge_home();
        let fabric = state.with_store(|s| {
            crate::cli::render_fabric_snapshot(
                s,
                scope,
                now,
                &self_slug,
                &self_pubkey,
                &host,
                &edge,
            )
        });
        if let Some(fabric) = fabric {
            out["fabric"] = serde_json::Value::String(fabric);
        }
        // Fold the current agent identity into `who` (issue #99): a `self` object
        // with this session's own fabric identity, present only when `who` runs
        // inside an agent. `session_id` is raw internal correlation, not a
        // user-facing identity.
        if let (Some(rec), Some(instance)) = (rec, instance.as_ref()) {
            let pending = state
                .with_store(|s| s.drain_pending_for_session(&rec.session_id))
                .map(|rows| rows.len())
                .unwrap_or(0);
            let is_member = state
                .with_store(|s| s.is_channel_member(&rec.channel_h, &instance.pubkey))
                .unwrap_or(true);
            out["self"] = serde_json::json!({
                "label": instance.display_slug(),
                "pubkey": instance.pubkey,
                "channel": rec.channel_h,
                "host": host,
                "is_member": is_member,
                "working": rec.working,
                "status": rec.title,
                "pending": pending,
                "created_at": rec.created_at,
                "session_id": rec.session_id,
            });
        }
    }
    Ok(out)
}

// ── project_add ──────────────────────────────────────────────────────────────
