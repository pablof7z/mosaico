use super::*;
use crate::session::{derive_status, DeltaKind, SessionSnapshot, StatusDeltaItem};

mod render;

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current project the
/// same way the old CLI did (`all_projects ? None : resolve(cwd)`).
fn who_params(project: &Option<String>, all_projects: bool) -> serde_json::Value {
    serde_json::json!({
        "project": project,
        "all_projects": all_projects,
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    })
}

fn who_snapshot_via_daemon(
    project: &Option<String>,
    all_projects: bool,
) -> Result<WhoSnapshot> {
    let v = crate::daemon::blocking::call("who", who_params(project, all_projects))?;
    Ok(serde_json::from_value(v)?)
}

pub(super) fn who(project: Option<String>, all_projects: bool) -> Result<()> {
    let snapshot = who_snapshot_via_daemon(&project, all_projects)?;
    print!("{}", render::render_who_for_stdout(&snapshot));
    Ok(())
}

pub(super) fn who_live(
    project: Option<String>,
    all_projects: bool,
) -> Result<()> {
    let refresh = Duration::from_millis(1000);
    let _terminal = render::LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let snapshot = who_snapshot_via_daemon(&project, all_projects)?;
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
/// cwd's project), then renders the same agent/channel/host vocabulary used by
/// `who` and the hook-injected fabric context.
pub(super) async fn whoami(session: Option<String>, json: bool) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": crate::cli::agent_env_slug(),
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
    now: u64,
    rows: Vec<WhoRow>,
    other_projects: Vec<OtherProjectSummary>,
    /// Agents tenex-edge has an identity for that can be spawned via tmux.
    #[serde(default)]
    spawnable: Vec<SpawnableRow>,
    /// When the current scope is a per-session room, the work-root project it is
    /// nested under. Lets the renderer label the room as the current *channel*
    /// (distinct from the *project*). `None` when the scope is a plain project.
    #[serde(default)]
    channel_parent: Option<String>,
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
    /// Hex pubkey others route to: the per-session pubkey when derived, else the
    /// durable agent pubkey.
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
    // Identity is (pubkey, group) now — a peer row that shares a live local
    // session's (pubkey, project) IS that session's own relay echo, so drop it.
    let my_keys: std::collections::HashSet<(String, String)> = mine
        .iter()
        .map(|s| (s.agent_pubkey.clone(), s.project.clone()))
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
                // The durable agent key is the session's wire identity now;
                // never prefer a stale per-session derived pubkey.
                pubkey: s.agent_pubkey.clone(),
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

/// Live `DerivedStatus` per agent pubkey in `channel` (local sessions preferred,
/// then peers), within the liveness window. Used to annotate channel members.
fn channel_status_map(
    store: &Store,
    channel: &str,
    now: u64,
) -> std::collections::HashMap<String, crate::session::DerivedStatus> {
    let since = now.saturating_sub(crate::session::STATUS_TTL_SECS);
    let mut map = std::collections::HashMap::new();
    // Peers first so a local session of the same agent overrides it.
    for snap in store
        .peer_session_snapshots(Some(channel), since)
        .unwrap_or_default()
    {
        map.insert(snap.agent_pubkey.clone(), crate::session::derive_status(&snap, now));
    }
    for snap in store
        .live_session_snapshots(Some(channel), since)
        .unwrap_or_default()
    {
        map.insert(snap.agent_pubkey.clone(), crate::session::derive_status(&snap, now));
    }
    map
}

/// Count of distinct LIVE agents in `channel` and whether any is busy.
fn channel_agent_activity(store: &Store, channel: &str, now: u64) -> (usize, bool) {
    let map = channel_status_map(store, channel, now);
    let live: Vec<_> = map
        .values()
        .filter(|ds| ds.liveness.is_live())
        .collect();
    let busy = live.iter().any(|ds| ds.busy);
    (live.len(), busy)
}

/// Render the channel-hierarchy context block injected at an agent's first turn.
/// Shows the agent's identity, where it sits in the channel tree, who else is in
/// the current channel, the subchannels beneath it, and a pointer to the rest of
/// the fabric. Returns `None` when there is no resolvable channel.
pub(super) fn render_channel_context(
    store: &Store,
    project: &str,
    now: u64,
    self_slug: &str,
    self_pubkey: &str,
) -> Option<String> {
    use std::fmt::Write as _;

    let breadcrumb = store.channel_breadcrumb(project).ok()?;
    if breadcrumb.is_empty() {
        return None;
    }
    let my_pubkey = self_pubkey;
    let my_slug = self_slug;
    let root_label = &breadcrumb[0].1;
    let leaf_label = breadcrumb
        .last()
        .map(|(_, label)| label.clone())
        .unwrap_or_default();
    let crumb = breadcrumb
        .iter()
        .map(|(_, label)| format!("#{label}"))
        .collect::<Vec<_>>()
        .join(" > ");

    let mut out = String::new();
    // Intro names the agent and its current channel (no session code — codenames
    // are an internal handle, not the agent's identity); the hierarchy follows.
    let _ = writeln!(
        out,
        "[tenex-edge] You are {my_slug} on #{leaf_label} — the channel hierarchy is shown below."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Project: {root_label}");
    let _ = writeln!(out, "Channel: {crumb}");
    if let Some(about) = store
        .get_project_meta(project)
        .ok()
        .flatten()
        .filter(|a| !a.is_empty())
    {
        let _ = writeln!(out, "Description: {about}");
    }

    // Members of the current channel, with their live activity.
    let members = store.list_group_members(project).unwrap_or_default();
    if !members.is_empty() {
        let status_map = channel_status_map(store, project, now);
        let mut parts: Vec<String> = Vec::new();
        for (pubkey, role) in &members {
            let you = if pubkey.as_str() == my_pubkey { " (you)" } else { "" };
            let label = match status_map.get(pubkey) {
                Some(ds) if ds.busy && !ds.activity.is_empty() => ds.activity.clone(),
                Some(_) => "idle".to_string(),
                // No live session: an admin with no agent identity reads as a
                // human; an agent member that is simply offline reads as such.
                None if role == "admin" => "Human".to_string(),
                None => "offline".to_string(),
            };
            let slug = store
                .resolve_slug_for_pubkey(pubkey)
                .ok()
                .flatten()
                .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
            parts.push(format!("@{slug}{you} - {label}"));
        }
        let _ = writeln!(out, "Members: {}", parts.join(" / "));
    }

    // Subchannels beneath the current channel, indented by depth.
    let subs = store.subchannels_of(project).unwrap_or_default();
    if !subs.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Subchannels:");
        for (id, name, depth) in &subs {
            let (count, busy) = channel_agent_activity(store, id, now);
            let indent = "  ".repeat(depth.saturating_sub(1));
            let agents = if count == 1 {
                "1 agent".to_string()
            } else {
                format!("{count} agents")
            };
            let status = if busy { "active" } else { "idle" };
            let _ = writeln!(out, "{indent}#{name} ({agents}) - {status}");
        }
    }

    // The rest of the fabric: how many other channels saw activity in 24h.
    let mut exclude: Vec<String> = vec![project.to_string()];
    exclude.extend(subs.iter().map(|(id, _, _)| id.clone()));
    let other = store
        .count_active_channels_since(now.saturating_sub(86_400), &exclude)
        .unwrap_or(0);
    if other > 0 {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "There {} {other} other active channel{} in the past 24 hours. \
             Use `tenex-edge channels list` for more.",
            if other == 1 { "is" } else { "are" },
            if other == 1 { "" } else { "s" }
        );
    }

    let _ = writeln!(out);
    let _ = write!(
        out,
        "To reach another agent, mention its `@name` (as listed above) in a \
         `tenex-edge chat write --message \"...\"` — run `tenex-edge chat write` whenever asked \
         to contact or notify another agent; do not say you cannot."
    );
    Some(out)
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
    self_slug: &str,
    self_pubkey: &str,
) {
    let store = store.lock().expect("store mutex poisoned");
    if first_turn {
        // The channel-hierarchy context: where the agent sits in the channel
        // tree, who shares its channel, the subchannels beneath, and a pointer
        // to the rest of the fabric.
        if let Some(block) = render_channel_context(&store, project, now, self_slug, self_pubkey) {
            blocks.push(block);
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
            daemon_host,
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
    daemon_host: &str,
    exclude_session: Option<&str>,
) -> Vec<String> {
    // Scope the delta to the current channel AND its subtree, so an agent sees
    // activity in subchannels beneath it (a new subchannel's first agent, a
    // sibling working below) — not just its own channel. Each subchannel's
    // display label is kept to tag cross-channel deltas.
    let subs = store.subchannels_of(project).unwrap_or_default();
    let mut channels: Vec<String> = Vec::with_capacity(subs.len() + 1);
    channels.push(project.to_string());
    channels.extend(subs.iter().map(|(id, _, _)| id.clone()));
    let labels: std::collections::HashMap<String, String> = subs
        .into_iter()
        .map(|(id, name, _)| (id, name))
        .collect();

    let items = store
        .status_delta_since_in(&channels, since, now, exclude_session)
        .unwrap_or_default();
    if items.is_empty() {
        return Vec::new();
    }

    let name_counts = delta_agent_name_counts(store, &items, project, now, daemon_host);

    // Canonical presence lines, one per change. master's name disambiguation
    // (delta_agent_label) is preserved; a delta from a subchannel additionally
    // gets a ` #<subchannel>` suffix so the agent knows where it happened.
    //   * bravo4217 (codex@laptop) joined
    //   * echo0163 (claude@tower) left #research
    //   * bravo4217 (codex@laptop) — reviewing the patch
    let mut delta: Vec<String> = Vec::with_capacity(items.len());
    for item in &items {
        let snap = &item.snapshot;
        let label = delta_agent_label(snap, &name_counts);
        let activity = render::status_plain("", &item.derived.activity, item.derived.busy);
        let suffix = if snap.project != project {
            let name = labels
                .get(snap.project.as_str())
                .cloned()
                .unwrap_or_else(|| snap.project.clone());
            format!(" #{name}")
        } else {
            String::new()
        };
        let line = match item.kind {
            DeltaKind::Appeared => format!("* {label} joined{suffix}"),
            DeltaKind::Gone => format!("* {label} left{suffix}"),
            DeltaKind::Changed => format!("* {label} — {activity}{suffix}"),
        };
        delta.push(line);
    }
    delta
}

fn delta_agent_label(
    snap: &SessionSnapshot,
    name_counts: &std::collections::BTreeMap<String, usize>,
) -> String {
    let agent = render::display_agent_name(
        snap.agent_slug.as_str(),
        snap.session_id.as_str(),
        name_counts,
    );
    let host = slugify_host(&snap.host);
    if host.is_empty() {
        agent
    } else {
        format!("{agent} ({host})")
    }
}

fn delta_agent_name_counts(
    store: &Store,
    items: &[StatusDeltaItem],
    project: &str,
    now: u64,
    daemon_host: &str,
) -> std::collections::BTreeMap<String, usize> {
    let mut seen = std::collections::BTreeSet::new();
    if let Ok(snapshot) = load_who_snapshot(store, Some(project), now, daemon_host) {
        for row in snapshot.rows {
            seen.insert((row.slug, row.session_id));
        }
    }
    for item in items {
        let snap = &item.snapshot;
        if snap.project == project {
            seen.insert((
                snap.agent_slug.clone(),
                snap.session_id.as_str().to_string(),
            ));
        }
    }

    let mut counts = std::collections::BTreeMap::new();
    for (slug, _) in seen {
        *counts.entry(slug).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests;
