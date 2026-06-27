use super::*;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct WhoParams {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    all_projects: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
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
    let current_project = if p.all_projects {
        None
    } else if p.project.is_none()
        && (p.env_session.as_deref().filter(|s| !s.is_empty()).is_some()
            || p.agent.as_deref().filter(|s| !s.is_empty()).is_some()
            || p.group.as_deref().filter(|s| !s.is_empty()).is_some())
    {
        Some(
            resolve_session_inner(
                state,
                None,
                p.env_session.as_deref(),
                p.cwd.as_deref(),
                p.agent.as_deref(),
                p.group.as_deref(),
                false,
            )
            .map(|rec| rec.route_scope().to_string())?,
        )
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
    Ok(serde_json::to_value(snapshot)?)
}

// ── session_start / session_end ──────────────────────────────────────────────

pub(in crate::daemon::server) fn rpc_whoami(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: StatuslineParams = serde_json::from_value(params.clone()).unwrap_or_default();
    // Strict: no bare-project fallback. `whoami` answers "which agent am I", so
    // when run outside an agent (no session/agent signal) it must error, not
    // silently report some unrelated sibling session in the cwd's project.
    let rec = resolve_session_inner(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        None,
        false,
    )?;
    let now = now_secs();
    let host = state.host.clone();
    // Routing scope: channel when set (a `channels switch` moved the session),
    // else the per-session room. `whoami` answers "which agent am I on the
    // fabric" — the scope it currently publishes into is the relevant one, and
    // the is_member check must key on it so a switched session doesn't report
    // a stale membership in the room it minted at spawn.
    let scope = rec.route_scope().to_string();
    state.with_store(|s| {
        // Report the session's DURABLE ORDINAL identity (issue #47): its ordinal
        // pubkey + label (`smith`, `smith1`, …) — that is the identity peers see
        // on the wire. Falls back to the legacy session pubkey, then the base
        // agent pubkey, for sessions predating the route table.
        let route = s.identity_route_for_session(&rec.session_id);
        let pubkey = route
            .as_ref()
            .map(|r| r.pubkey.clone())
            .or_else(|| s.session_pubkey_for_session(&rec.session_id))
            .unwrap_or_else(|| rec.agent_pubkey.clone());
        let label = route
            .as_ref()
            .map(|r| r.label.clone())
            .unwrap_or_else(|| rec.agent_slug.clone());
        let npub = {
            use nostr_sdk::prelude::ToBech32;
            nostr_sdk::PublicKey::from_hex(&pubkey)
                .ok()
                .and_then(|pk| pk.to_bech32().ok())
        };
        let is_member = s.is_group_member(&scope, &pubkey).unwrap_or(true);
        let (working, status) = s
            .local_session_snapshot(&rec.session_id)
            .ok()
            .flatten()
            .map(|snap| {
                let d = derive_status(&snap, now);
                (d.busy, d.title)
            })
            .unwrap_or((false, String::new()));
        let pending = s
            .peek_chat_mentions(&rec.session_id)
            .unwrap_or_default()
            .len();
        Ok(serde_json::json!({
            "agent": rec.agent_slug,
            "label": label,
            "session_id": rec.session_id,
            "codename": crate::util::session_codename(&rec.session_id),
            "project": scope,
            "host": host,
            "rel_cwd": rec.rel_cwd,
            "pubkey": pubkey,
            "npub": npub,
            "is_member": is_member,
            "working": working,
            "status": status,
            "pending": pending,
            "created_at": rec.created_at,
        }))
    })
}

// ── project_add ──────────────────────────────────────────────────────────────
