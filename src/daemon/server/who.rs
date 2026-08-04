use super::*;

#[path = "who/human.rs"]
mod human_view;

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct WhoParams {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    all_workspaces: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    human_color: bool,
    /// `who --expired`: list this machine's stopped sessions by public handle so a
    /// user can pick one to resume, instead of the live fabric snapshot.
    #[serde(default)]
    expired: bool,
}

/// Cap on the expired-session listing — the resume-candidate window.
const EXPIRED_SESSION_LIMIT: u32 = 100;

/// Operator-oriented fabric overview. The command stays hidden from default
/// agent help, but an explicit invocation returns the same read-only view.
pub(in crate::daemon::server) fn rpc_who(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: WhoParams = serde_json::from_value(params.clone()).unwrap_or_default();
    if p.expired {
        let host = state.host.clone();
        let rows = state.with_store(|s| {
            crate::expired_sessions::load_expired_sessions(s, &host, EXPIRED_SESSION_LIMIT)
        });
        return Ok(serde_json::json!({ "expired": rows }));
    }
    let current_root = if p.all_workspaces {
        None
    } else {
        Some(if let Some(channel) = p.channel.as_deref() {
            super::channel_membership_rpc::resolve_target_channel(state, channel)?
        } else {
            match p.workspace.clone() {
                Some(workspace) => workspace,
                None => {
                    let cwd = p
                        .cwd
                        .clone()
                        .map(std::path::PathBuf::from)
                        .map(Ok)
                        .unwrap_or_else(std::env::current_dir)?;
                    crate::daemon::workspace_path::channel_for_path(&cwd)?
                }
            }
        })
    };
    let now = now_secs();
    let host = state.host.clone();
    // This daemon's own management pubkey, excluded from every rendered roster so
    // the backend key never appears as a channel member (its kind:0 is absent on a
    // cold cache, so identity — not a fetched profile — is the reliable signal).
    let backend_pk = state.backend_pubkey().unwrap_or_default();
    let mut snapshot = state.with_store(|s| {
        crate::who_snapshot::load_who_snapshot(s, current_root.as_deref(), now, &host)
    })?;
    state.with_store(|store| publicize_snapshot(store, &mut snapshot));
    let mut out = serde_json::to_value(&snapshot)?;

    if let Some(scope) = current_root.as_deref() {
        let human = state.with_store(|s| {
            crate::fabric_context::render_fabric_context_human_scoped(
                s,
                crate::fabric_context::FabricContextInput {
                    session: None,
                    scope,
                    cursor: 0,
                    now,
                    self_slug: "",
                    self_pubkey: "",
                    backend_pubkey: &backend_pk,
                    local_host: &host,
                    forced_messages: &[],
                    warnings: &[],
                    force: true,
                },
                p.human_color,
            )
        })?;
        if let Some(mut human) = human {
            human_view::append_other_roots(&mut human, &snapshot.other_roots, p.human_color);
            out["fabric_human"] = serde_json::Value::String(human);
        }
    } else if p.all_workspaces {
        let roots = state.with_store(all_known_roots)?;
        let human = state.with_store(|s| {
            crate::fabric_context::render_fabric_all_workspaces_human(
                s,
                &roots,
                now,
                &host,
                &backend_pk,
                p.human_color,
            )
        })?;
        out["fabric_human"] = serde_json::Value::String(human);
    }
    Ok(out)
}

fn publicize_snapshot(
    store: &crate::state::Store,
    snapshot: &mut crate::who_snapshot::WhoSnapshot,
) {
    let public = |channel: &str| {
        if channel == "*" {
            "*".to_string()
        } else {
            crate::channel_ref::full_channel_ref(store, channel)
        }
    };
    snapshot.rows.retain_mut(|row| {
        row.channel = public(&row.channel);
        row.work_root = public(&row.work_root);
        row.work_root_display = row.work_root.clone();
        !row.channel.is_empty() && !row.work_root.is_empty()
    });
    snapshot.other_roots.retain_mut(|root| {
        root.root = public(&root.root);
        !root.root.is_empty()
    });
    snapshot.channel_parent = snapshot
        .channel_parent
        .take()
        .map(|parent| public(&parent))
        .filter(|parent| !parent.is_empty());
    snapshot.root = public(&snapshot.root);
    snapshot.root_display = snapshot.root.clone();
}

/// Every non-archived top-level root this daemon knows of, plus every locally
/// bound workspace — the set `who --all-workspaces` fans its unified fabric
/// render across.
///
/// Deliberately "everything cached", because that is what the operator asked
/// to see. It is NOT the set this backend manages, and must not be reused as
/// one: that question is answered by the relay-signed admin lists
/// (`backend_profile::managed_roots`), not by what happens to be cached.
fn all_known_roots(store: &crate::state::Store) -> Result<Vec<String>> {
    let mut roots = store
        .reader()
        .list_channels()?
        .into_iter()
        .filter(|c| c.parent.is_empty() && !c.is_archived())
        .map(|c| c.channel_h)
        .collect::<std::collections::BTreeSet<_>>();
    roots.extend(
        crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .bindings()?
            .into_iter()
            .map(|binding| binding.channel_h),
    );
    Ok(roots.into_iter().collect())
}

#[cfg(test)]
mod tests;
