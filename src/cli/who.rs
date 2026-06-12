use super::*;

mod render;

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current project the
/// same way the old CLI did (`all_projects ? None : resolve(cwd)`).
fn who_params(project: &Option<String>, all: bool, all_projects: bool) -> serde_json::Value {
    serde_json::json!({
        "project": project,
        "all": all,
        "all_projects": all_projects,
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    })
}

fn who_snapshot_via_daemon(
    project: &Option<String>,
    all: bool,
    all_projects: bool,
) -> Result<WhoSnapshot> {
    let v = crate::daemon::blocking::call("who", who_params(project, all, all_projects))?;
    Ok(serde_json::from_value(v)?)
}

pub(super) fn who(project: Option<String>, all: bool, all_projects: bool) -> Result<()> {
    let snapshot = who_snapshot_via_daemon(&project, all, all_projects)?;
    print!("{}", render::render_who_once(&snapshot));
    Ok(())
}

pub(super) fn who_live(
    project: Option<String>,
    all: bool,
    all_projects: bool,
    refresh: Duration,
) -> Result<()> {
    let refresh = refresh.max(Duration::from_millis(100));
    let _terminal = render::LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let snapshot = who_snapshot_via_daemon(&project, all, all_projects)?;
            render::draw_who_live(&snapshot, refresh)?;
            next_draw = Instant::now() + refresh;
        }

        let wait = next_draw
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        if event::poll(wait)? {
            if render::should_quit_live(event::read()?) {
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OtherProjectSummary {
    project: String,
    agent_count: usize,
    #[serde(default)]
    agents: Vec<String>,
    about: Option<String>,
}

// The daemon serializes a WhoSnapshot and the thin `who` client renders it with
// the EXACT renderers below — so output is byte-identical by construction and
// can never drift from a separate copy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WhoSnapshot {
    project: String,
    all: bool,
    now: u64,
    rows: Vec<WhoRow>,
    other_projects: Vec<OtherProjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WhoRow {
    source: WhoSource,
    fresh: bool,
    slug: String,
    project: String,
    status: String,
    host: String,
    session_id: SessionId,
    age_secs: Option<u64>,
    /// Project-relative working dir (§8e). Empty or "." → rendered without a
    /// `[dir]` bracket; otherwise shown so worktrees render distinctly.
    #[serde(default)]
    rel_cwd: String,
    /// True only for a peer whose host differs from the daemon/viewer's host.
    /// Local sessions and same-machine peers are never remote (the §8e fix).
    #[serde(default)]
    remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum WhoSource {
    Local,
    Peer,
}

pub fn load_who_snapshot(
    store: &Store,
    current_project: Option<&str>,
    all: bool,
    now: u64,
    daemon_host: &str,
) -> Result<WhoSnapshot> {
    // §8e: "remote" is computed DAEMON-side by comparing each peer's host to the
    // daemon's own host, so all rendering stays client-side and can't diverge via
    // a second Config::load(). Local sessions are on this machine by construction
    // → never remote. A peer is remote ONLY when its host differs from ours.
    let local_host = slugify_host(daemon_host);
    let since = if all {
        0
    } else {
        now.saturating_sub(PEER_FRESH_SECS)
    };

    let mine = store.list_my_live_sessions(since)?;
    let my_ids: std::collections::HashSet<String> =
        mine.iter().map(|s| s.session_id.clone()).collect();
    let local_agent_pubkeys: std::collections::HashSet<String> = store
        .list_local_agent_pubkeys()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let all_peers: Vec<_> = store
        .list_peer_sessions(None, since)?
        .into_iter()
        .filter(|p| !my_ids.contains(&p.session_id))
        .filter(|p| {
            !(slugify_host(&p.host) == local_host && local_agent_pubkeys.contains(&p.pubkey))
        })
        .collect();

    let mut rows = Vec::new();
    let mut other_agents: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();

    for s in &mine {
        let age_secs = store
            .session_last_seen(&s.session_id)
            .ok()
            .flatten()
            .map(|ls| now.saturating_sub(ls));
        if current_project.map(|p| p == s.project).unwrap_or(true) {
            rows.push(WhoRow {
                source: WhoSource::Local,
                fresh: age_secs.map(|a| a <= PEER_FRESH_SECS).unwrap_or(true),
                slug: s.agent_slug.clone(),
                project: s.project.clone(),
                status: status_for(store, &s.agent_pubkey, &s.project, Some(&s.session_id)),
                host: s.host.clone(),
                session_id: SessionId::from(s.session_id.clone()),
                age_secs,
                rel_cwd: s.rel_cwd.clone(),
                remote: false,
            });
        } else {
            other_agents
                .entry(s.project.clone())
                .or_default()
                .insert(s.agent_slug.clone());
        }
    }

    for p in &all_peers {
        let age = now.saturating_sub(p.last_seen);
        if current_project.map(|cp| cp == p.project).unwrap_or(true) {
            rows.push(WhoRow {
                source: WhoSource::Peer,
                fresh: age <= PEER_FRESH_SECS,
                slug: p.slug.clone(),
                project: p.project.clone(),
                status: status_for(store, &p.pubkey, &p.project, Some(&p.session_id)),
                host: p.host.clone(),
                session_id: SessionId::from(p.session_id.clone()),
                age_secs: Some(age),
                rel_cwd: p.rel_cwd.clone(),
                remote: slugify_host(&p.host) != local_host,
            });
        } else {
            other_agents
                .entry(p.project.clone())
                .or_default()
                .insert(p.slug.clone());
        }
    }

    let other_projects = other_agents
        .into_iter()
        .map(|(project, agents)| {
            let about = store.get_project_meta(&project).ok().flatten();
            let agents: Vec<String> = agents.into_iter().collect();
            OtherProjectSummary {
                project,
                agent_count: agents.len(),
                agents,
                about,
            }
        })
        .collect();

    Ok(WhoSnapshot {
        project: current_project.unwrap_or("*").to_string(),
        all,
        now,
        rows,
        other_projects,
    })
}

fn status_for(store: &Store, pubkey: &str, project: &str, session_id: Option<&str>) -> String {
    store
        .get_agent_status(pubkey, project, session_id)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Append the turn-start "tenex-edge fabric" block(s): the full roster on the
/// first turn, or "changes since your last turn" afterward. This is the single
/// source of truth — both the CLI `turn_start` and the daemon's `turn_start` RPC
/// call it, so the injected text is identical.
pub(super) fn push_turn_fabric_block(
    store: &std::sync::Mutex<Store>,
    blocks: &mut Vec<String>,
    first_turn: bool,
    prev_turn_started_at: u64,
    project: &str,
    now: u64,
    daemon_host: &str,
) {
    let store = store.lock().expect("store mutex poisoned");
    if first_turn {
        if let Ok(snapshot) = load_who_snapshot(&store, Some(project), false, now, daemon_host) {
            if !snapshot.rows.is_empty() {
                let who_text = render::render_who_plain(&snapshot);
                blocks.push(format!(
                "tenex-edge fabric — agents you can message. To send, run \
                 `tenex-edge send-message --recipient <agent@project|session-id> --message \"...\"`:\n{}",
                who_text.trim_end()
            ));
            }
        }
    } else {
        let fresh_since = now.saturating_sub(PEER_FRESH_SECS);
        let new_peers = store
            .list_new_peer_sessions(prev_turn_started_at, fresh_since, Some(project))
            .unwrap_or_default();
        let status_changes = store
            .list_status_changes_since(prev_turn_started_at, Some(project))
            .unwrap_or_default();

        let mut delta: Vec<String> = Vec::new();
        for p in &new_peers {
            let age = now.saturating_sub(p.last_seen);
            delta.push(format!(
                "  ● {}@{} joined  {}  session {}  ({age}s ago)",
                p.slug,
                slugify_host(&p.host),
                p.project,
                SessionId::from(p.session_id.as_str()),
            ));
        }
        for (slug, proj, text) in &status_changes {
            delta.push(format!("  ↻ {slug}@{proj} — {text}"));
        }
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes since your last turn:\n{}",
                delta.join("\n")
            ));
        }
    }
}

#[cfg(test)]
mod tests;
