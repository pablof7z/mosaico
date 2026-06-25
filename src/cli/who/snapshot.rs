use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct OtherProjectSummary {
    pub(super) project: String,
    pub(super) agent_count: usize,
    #[serde(default)]
    pub(super) agents: Vec<String>,
    pub(super) about: Option<String>,
}

// The daemon serializes a WhoSnapshot and the thin `who` client renders it with
// the EXACT renderers below — so output is byte-identical by construction and
// can never drift from a separate copy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WhoSnapshot {
    pub(super) project: String,
    pub(super) now: u64,
    pub(super) rows: Vec<WhoRow>,
    pub(super) other_projects: Vec<OtherProjectSummary>,
    /// Agents tenex-edge has an identity for that can be spawned via tmux.
    #[serde(default)]
    pub(super) spawnable: Vec<SpawnableRow>,
    /// When the current scope is a per-session room, the work-root project it is
    /// nested under. Lets the renderer label the room as the current *channel*
    /// (distinct from the *project*). `None` when the scope is a plain project.
    #[serde(default)]
    pub(super) channel_parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct SpawnableRow {
    pub(super) host: String,
    pub(super) slug: String,
    pub(super) command: String,
    /// Optional one-line "when to use this agent" note from the agent file.
    #[serde(default)]
    pub(super) byline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct WhoRow {
    pub(super) source: WhoSource,
    pub(super) fresh: bool,
    pub(super) slug: String,
    pub(super) project: String,
    /// Persistent session title (what the session is about); survives idle turns.
    pub(super) status: String,
    /// Live "doing now" line, distilled alongside the title. Shown after the
    /// title while mid-turn; empty (and not rendered) when idle.
    #[serde(default)]
    pub(super) activity: String,
    /// Whether the session is mid-turn. Drives the idle marker independently of
    /// the title, which is retained while idle.
    #[serde(default)]
    pub(super) active: bool,
    pub(super) host: String,
    pub(super) session_id: String,
    pub(super) age_secs: Option<u64>,
    /// Project-relative working dir (§8e). Empty or "." → rendered without a
    /// `[dir]` bracket; otherwise shown so worktrees render distinctly.
    #[serde(default)]
    pub(super) rel_cwd: String,
    /// True only for a peer whose host differs from the daemon/viewer's host.
    /// Local sessions and same-machine peers are never remote (the §8e fix).
    #[serde(default)]
    pub(super) remote: bool,
    /// True when this session has a live tmux endpoint registered — i.e. it
    /// can be attached to via `tenex-edge tmux attach`.
    #[serde(default)]
    pub(super) attachable: bool,
    /// Top-level work-root project for UI grouping. `project` remains the live
    /// routing scope (session room or task channel); this is the project tab.
    #[serde(default)]
    pub(super) work_root: String,
    /// Hex pubkey others route to: the per-session pubkey when derived, else the
    /// durable agent pubkey.
    #[serde(default)]
    pub(super) pubkey: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum WhoSource {
    Local,
    Peer,
}

pub fn load_who_snapshot(
    store: &Store,
    current_project: Option<&str>,
    now: u64,
    daemon_host: &str,
) -> Result<WhoSnapshot> {
    // §8e: "remote" is computed DAEMON-side by comparing each peer's host to the
    // daemon's own host, so all rendering stays client-side and can't diverge via
    // a second Config::load(). Local sessions are on this machine by construction
    // → never remote. A peer is remote ONLY when its host differs from ours.
    let local_host = slugify_host(daemon_host);
    let since = now.saturating_sub(PEER_FRESH_SECS);

    // Single source of truth: the session-state read facade. Local rows project
    // `session_state`, peer rows project `peer_session_state`, and BOTH run through
    // the one `derive_status` projection — there is no local-vs-peer busy fork.
    let mine = store.live_session_snapshots(None, since)?;
    // Identity is (signing pubkey, group) now. A normal session signs with the
    // durable agent key; a collision-fallback duplicate signs with its
    // session_pubkey. Peer rows sharing that selected local identity are our own
    // relay echoes, so drop them.
    let my_keys: std::collections::HashSet<(String, String)> = mine
        .iter()
        .map(|s| {
            (
                store
                    .session_pubkey_for_session(s.session_id.as_str())
                    .unwrap_or_else(|| s.agent_pubkey.clone()),
                s.project.clone(),
            )
        })
        .collect();
    let local_agent_pubkeys: std::collections::HashSet<String> = store
        .list_local_agent_pubkeys()
        .unwrap_or_default()
        .into_iter()
        .collect();
    let all_peers: Vec<SessionSnapshot> = store
        .peer_session_snapshots(None, since)?
        .into_iter()
        .filter(|p| !my_keys.contains(&(p.agent_pubkey.clone(), p.project.clone())))
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
            let session_pubkey = store.session_pubkey_for_session(sid);
            let display_pubkey = session_pubkey
                .clone()
                .unwrap_or_else(|| s.agent_pubkey.clone());
            let display_slug = if session_pubkey.is_some() {
                format!("{} ({})", session_codename(sid), s.agent_slug)
            } else {
                s.agent_slug.clone()
            };
            rows.push(WhoRow {
                source: WhoSource::Local,
                fresh: d.liveness.is_live(),
                slug: display_slug,
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
                work_root: store
                    .work_root_for_scope(&s.project)
                    .unwrap_or_else(|_| s.project.clone()),
                pubkey: display_pubkey,
            });
        } else if store.is_root_project(&s.project) {
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
                work_root: store
                    .work_root_for_scope(&p.project)
                    .unwrap_or_else(|_| p.project.clone()),
                // Peer status is session-signed, so agent_pubkey IS the peer's
                // session pubkey — the address to route to.
                pubkey: p.agent_pubkey.clone(),
            });
        } else if store.is_root_project(&p.project) {
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

    // If the current scope is a per-session room, surface its work-root parent
    // so the renderer can label it as the channel (not the project).
    let channel_parent = current_project.and_then(|p| store.session_room_parent(p).ok().flatten());

    Ok(WhoSnapshot {
        project: current_project.unwrap_or("*").to_string(),
        now,
        rows,
        other_projects,
        spawnable,
        channel_parent,
    })
}

impl WhoSnapshot {
    /// Number of visible sessions (local + peer, including idle) in scope.
    pub fn session_count(&self) -> usize {
        self.rows.len()
    }
}
