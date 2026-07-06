use super::*;
use crate::who_snapshot::OtherProjectSummary;
use owo_colors::OwoColorize as _;
use std::fmt::Write as _;

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
    #[serde(default)]
    human_color: bool,
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
    let snapshot = state.with_store(|s| {
        crate::who_snapshot::load_who_snapshot(s, current_project.as_deref(), now, &host)
    })?;
    let mut out = serde_json::to_value(&snapshot)?;

    // Attach the UNIFIED fabric view (same format as the hook injection — decision
    // A) whenever a single current channel resolves. `--all-projects` has no single
    // scope, so it keeps the cross-project snapshot table. The caller (this session,
    // when run inside an agent) is marked `(you)` and excluded from peer echoes.
    if let Some(scope) = current_project.as_deref() {
        // Reuse the exact caller session, when present, for both the fabric
        // `(you)` match and the folded-in `<self />` row (issue #99).
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
        let fabric = state.with_store(|s| {
            crate::fabric_context::render_fabric_context(
                s,
                crate::fabric_context::FabricContextInput {
                    session: rec,
                    scope,
                    cursor: rec.map(|r| r.seen_cursor).unwrap_or(0),
                    now,
                    self_slug: &self_slug,
                    self_pubkey: &self_pubkey,
                    local_host: &host,
                    forced_messages: &[],
                    warnings: &[],
                    force: true,
                },
            )
        });
        if let Some(fabric) = fabric {
            out["fabric"] = serde_json::Value::String(fabric);
            if rec.is_none() {
                let human = state.with_store(|s| {
                    crate::fabric_context::render_fabric_context_human(
                        s,
                        crate::fabric_context::FabricContextInput {
                            session: rec,
                            scope,
                            cursor: 0,
                            now,
                            self_slug: &self_slug,
                            self_pubkey: &self_pubkey,
                            local_host: &host,
                            forced_messages: &[],
                            warnings: &[],
                            force: true,
                        },
                        p.human_color,
                    )
                });
                if let Some(mut human) = human {
                    append_other_projects_human(
                        &mut human,
                        &snapshot.other_projects,
                        p.human_color,
                    );
                    out["fabric_human"] = serde_json::Value::String(human);
                }
            }
            if let Some(rec) = rec {
                if let Err(e) = cursor::drive_cursor_request(
                    state,
                    "who",
                    cursor::seed_from_session(rec),
                    cursor::fact_from_session(rec, now, true),
                ) {
                    tracing::error!(
                        session = %rec.session_id,
                        error = ?e,
                        "who: advancing session fabric cursor failed"
                    );
                }
            }
        }
    } else if p.all_projects {
        // No single scope exists across all projects, so `--all-projects` gets
        // the same fabric renderer applied once per root project instead of
        // falling back to the old snapshot table (issue: `who` and
        // `who --all-projects` must not diverge in output format).
        let roots = state.with_store(project_roots)?;
        let fabric = state.with_store(|s| {
            crate::fabric_context::render_fabric_all_projects(s, &roots, now, &host)
        });
        out["fabric"] = serde_json::Value::String(fabric);
        let human = state.with_store(|s| {
            crate::fabric_context::render_fabric_all_projects_human(
                s,
                &roots,
                now,
                &host,
                p.human_color,
            )
        });
        out["fabric_human"] = serde_json::Value::String(human);
    }
    Ok(out)
}

/// Top-level project channels (`parent` empty), non-archived — the set
/// `--all-projects` fans its unified fabric render across.
fn project_roots(store: &crate::state::Store) -> Result<Vec<String>> {
    Ok(store
        .reader()
        .list_channels()?
        .into_iter()
        .filter(|c| c.parent.is_empty() && !c.is_archived())
        .map(|c| c.channel_h)
        .collect())
}

fn append_other_projects_human(
    out: &mut String,
    other_projects: &[OtherProjectSummary],
    color: bool,
) {
    if other_projects.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "{}",
        human_style("Other projects", color, HumanStyle::Header)
    );
    for project in other_projects {
        let name = human_style(&project.project, color, HumanStyle::Project);
        let agents = project
            .agents
            .iter()
            .map(|agent| human_style(&format!("@{agent}"), color, HumanStyle::Agent))
            .collect::<Vec<_>>()
            .join(", ");
        let count = format!(
            "{} agent{}",
            project.agent_count,
            if project.agent_count == 1 { "" } else { "s" }
        );
        let about = project
            .about
            .as_deref()
            .filter(|about| !about.trim().is_empty())
            .map(|about| format!(" - {about}"))
            .unwrap_or_default();
        if agents.is_empty() {
            let _ = writeln!(out, "  {}  {}{}", name, human_dim(&count, color), about);
        } else {
            let _ = writeln!(
                out,
                "  {}  {}  {}{}",
                name,
                human_dim(&count, color),
                agents,
                about
            );
        }
    }
    out.push('\n');
}

#[derive(Clone, Copy)]
enum HumanStyle {
    Agent,
    Header,
    Project,
}

fn human_style(text: &str, color: bool, style: HumanStyle) -> String {
    if !color {
        return text.to_string();
    }
    match style {
        HumanStyle::Agent => text.cyan().to_string(),
        HumanStyle::Header => text.bold().to_string(),
        HumanStyle::Project => text.blue().bold().to_string(),
    }
}

fn human_dim(text: &str, color: bool) -> String {
    if color {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

// ── project_add ──────────────────────────────────────────────────────────────
