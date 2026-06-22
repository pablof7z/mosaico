use super::*;
use crate::session::{derive_status, DeltaKind, SessionSnapshot};

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
    print!("{}", render::render_who_for_stdout(&snapshot));
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
        if event::poll(wait)? && render::should_quit_live(event::read()?) {
            break;
        }
    }

    Ok(())
}

/// `whoami`: print this session's own identity card. Resolves the current
/// session daemon-side (explicit `--session` → `TENEX_EDGE_SESSION` env → the
/// cwd's project), then renders who you are on the fabric so an agent can pick
/// its own row out of `who` and knows the codename others address it by.
pub(super) async fn whoami(session: Option<String>, json: bool) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": crate::cli::agent_env_slug(),
        "group": crate::cli::group_env(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = super::daemon_call_async("whoami", params).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print!("{}", render::render_whoami(&v));
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
    /// Agents tenex-edge has an identity for that can be spawned via tmux.
    #[serde(default)]
    spawnable: Vec<SpawnableRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SpawnableRow {
    host: String,
    slug: String,
    command: String,
    /// Optional one-line "when to use this agent" note from the agent file.
    #[serde(default)]
    byline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WhoRow {
    source: WhoSource,
    fresh: bool,
    slug: String,
    project: String,
    /// Persistent session title (what the session is about); survives idle turns.
    status: String,
    /// Live "doing now" line, distilled alongside the title. Shown after the
    /// title while mid-turn; empty (and not rendered) when idle.
    #[serde(default)]
    activity: String,
    /// Whether the session is mid-turn. Drives the idle marker independently of
    /// the title, which is retained while idle.
    #[serde(default)]
    active: bool,
    host: String,
    session_id: String,
    age_secs: Option<u64>,
    /// Project-relative working dir (§8e). Empty or "." → rendered without a
    /// `[dir]` bracket; otherwise shown so worktrees render distinctly.
    #[serde(default)]
    rel_cwd: String,
    /// True only for a peer whose host differs from the daemon/viewer's host.
    /// Local sessions and same-machine peers are never remote (the §8e fix).
    #[serde(default)]
    remote: bool,
    /// True when this session has a live tmux endpoint registered — i.e. it
    /// can be attached to via `tenex-edge tmux attach`.
    #[serde(default)]
    attachable: bool,
    /// Number of unread inbox mentions for this session.
    #[serde(default)]
    unread: usize,
    /// Hex pubkey others route to: the per-session pubkey when derived, else the
    /// durable agent pubkey. This is the wire address behind the codename.
    #[serde(default)]
    pubkey: String,
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

    // Single source of truth: the session-state read facade. Local rows project
    // `session_state`, peer rows project `peer_session_state`, and BOTH run through
    // the one `derive_status` projection — there is no local-vs-peer busy fork.
    let mine = store.live_session_snapshots(None, since)?;
    let my_ids: std::collections::HashSet<String> = mine
        .iter()
        .map(|s| s.session_id.as_str().to_string())
        .collect();
    let local_agent_pubkeys: std::collections::HashSet<String> = store
        .list_local_agent_pubkeys()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let all_peers: Vec<SessionSnapshot> = store
        .peer_session_snapshots(None, since)?
        .into_iter()
        .filter(|p| !my_ids.contains(p.session_id.as_str()))
        .filter(|p| {
            !(slugify_host(&p.host) == local_host && local_agent_pubkeys.contains(&p.agent_pubkey))
        })
        .collect();

    // Sessions that have a tmux endpoint registered (for attachable flag).
    let tmux_sessions: std::collections::HashSet<String> = store
        .list_session_endpoints_of_kind("tmux")
        .unwrap_or_default()
        .into_iter()
        .map(|ep| ep.session_id)
        .collect();

    let mut rows = Vec::new();
    let mut other_agents: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();

    for s in &mine {
        let sid = s.session_id.as_str();
        if current_project.map(|p| p == s.project).unwrap_or(true) {
            // Derived projection: busy/liveness/title/activity all come from the
            // one `derive_status` so idle/freshness can never diverge per source.
            let d = derive_status(s, now);
            let unread = store.count_unread_inbox(sid).unwrap_or(0);
            rows.push(WhoRow {
                source: WhoSource::Local,
                fresh: d.liveness.is_live(),
                slug: s.agent_slug.clone(),
                project: s.project.clone(),
                status: d.title,
                activity: d.activity,
                active: d.busy,
                host: s.host.clone(),
                session_id: sid.to_string(),
                age_secs: Some(d.age_secs),
                rel_cwd: s.rel_cwd.clone(),
                remote: false,
                attachable: tmux_sessions.contains(sid),
                unread,
                pubkey: store
                    .session_pubkey_for_session(sid)
                    .unwrap_or_else(|| s.agent_pubkey.clone()),
            });
        } else {
            other_agents
                .entry(s.project.clone())
                .or_default()
                .insert(s.agent_slug.clone());
        }
    }

    for p in &all_peers {
        let sid = p.session_id.as_str();
        if current_project.map(|cp| cp == p.project).unwrap_or(true) {
            // Identical derivation path as local rows — the fork is gone.
            let d = derive_status(p, now);
            rows.push(WhoRow {
                source: WhoSource::Peer,
                fresh: d.liveness.is_live(),
                slug: p.agent_slug.clone(),
                project: p.project.clone(),
                status: d.title,
                activity: d.activity,
                active: d.busy,
                host: p.host.clone(),
                session_id: sid.to_string(),
                age_secs: Some(d.age_secs),
                rel_cwd: p.rel_cwd.clone(),
                remote: slugify_host(&p.host) != local_host,
                attachable: false,
                unread: 0,
                // Peer status is session-signed, so agent_pubkey IS the peer's
                // session pubkey — the address to route to.
                pubkey: p.agent_pubkey.clone(),
            });
        } else {
            other_agents
                .entry(p.project.clone())
                .or_default()
                .insert(p.agent_slug.clone());
        }
    }

    let other_projects = other_agents
        .into_iter()
        .map(|(project, agents)| {
            // Route through the read-model method so Phase 8 can swap the source.
            let about = store.project_meta_read_model(&project).ok().flatten();
            let agents: Vec<String> = agents.into_iter().collect();
            OtherProjectSummary {
                project,
                agent_count: agents.len(),
                agents,
                about,
            }
        })
        .collect();

    let spawnable: Vec<SpawnableRow> = crate::tmux::spawnable_agents()
        .into_iter()
        .map(|(slug, command, byline)| SpawnableRow {
            host: local_host.clone(),
            slug,
            command,
            byline,
        })
        .collect();

    Ok(WhoSnapshot {
        project: current_project.unwrap_or("*").to_string(),
        all,
        now,
        rows,
        other_projects,
        spawnable,
    })
}

impl WhoSnapshot {
    /// Number of visible sessions (local + peer, including idle) in scope.
    pub fn session_count(&self) -> usize {
        self.rows.len()
    }
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
    self_session: &str,
) {
    let store = store.lock().expect("store mutex poisoned");
    if first_turn {
        if let Ok(snapshot) = load_who_snapshot(&store, Some(project), false, now, daemon_host) {
            if !snapshot.rows.is_empty() {
                let who_text = render::render_who_plain(&snapshot);
                blocks.push(format!(
                "tenex-edge fabric — agents you can message. Message an existing session with \
                 `tenex-edge inbox send --to-session <codename> --subject \"...\" --message \"...\"`, \
                 or start a fresh one with `tenex-edge inbox send --to-new-session <agent> ...`:\n{}",
                who_text.trim_end()
            ));
            }
        }
    } else {
        // Self-exclude the viewer's own session: rpc_turn_start opens this turn
        // (busy transition) BEFORE context assembly, so without this the session
        // would see its own just-started change echoed back as a delta.
        let delta = build_status_delta(
            &store,
            prev_turn_started_at,
            project,
            now,
            Some(self_session),
        );
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes since your last turn:\n{}",
                delta.join("\n")
            ));
        }
    }
}

/// Build the "changes since X" delta lines from the single shared delta query.
/// Every in-scope session (local AND peer) is classified by `status_delta_since`
/// into exactly one of appeared / changed / gone since `since`, project-scoped,
/// with `exclude_session` (the viewer's own session) filtered out at the source.
/// Shared by the turn-start delta (subsequent turns) and the mid-turn PostToolUse
/// check, so both render identically.
///
/// - Appeared: a session that joined since the cursor (`● … joined`).
/// - Changed:  a versioned content change — the agent finished (busy→idle) or a
///   new title landed (`↻ … — <status>`).
/// - Gone:     the session ended/was superseded, or its liveness expired in the
///   window (`✗ … left`). A dropped-off session stays reportable as gone.
pub(super) fn build_status_delta(
    store: &Store,
    since: u64,
    project: &str,
    now: u64,
    exclude_session: Option<&str>,
) -> Vec<String> {
    let items = store
        .status_delta_since(project, since, now, exclude_session)
        .unwrap_or_default();
    if items.is_empty() {
        return Vec::new();
    }

    // Render the delta with the SAME markdown table shape as the first-turn
    // roster (`render_who_plain`) so both surfaces look identical. The change
    // kind (joined / changed / left) is folded into the Status cell.
    let mut delta: Vec<String> = render::SESSION_TABLE_HEADER
        .iter()
        .map(|s| s.to_string())
        .collect();
    for item in &items {
        let snap = &item.snapshot;
        let d = &item.derived;
        let slug = snap.agent_slug.as_str();
        let code = session_codename(snap.session_id.as_str());
        let title = if d.title.trim().is_empty() {
            "—".to_string()
        } else {
            d.title.trim().to_string()
        };
        // Live activity / idle word, without the title (that's its own column).
        let activity = render::status_plain("", &d.activity, d.busy);
        let status = match item.kind {
            DeltaKind::Appeared => format!("joined · {activity}"),
            DeltaKind::Changed => activity,
            DeltaKind::Gone => "left".to_string(),
        };
        delta.push(render::session_table_row(slug, &code, &snap.host, &title, &status));
    }
    delta
}

#[cfg(test)]
mod tests;
