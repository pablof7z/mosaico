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
    /// The human DISPLAY label for `project`: its kind:39000 `name` when set, else
    /// the raw scope id. Rendered in the `Channel:`/`Project:` headers so the
    /// opaque channel id never surfaces when a name exists. `*` for all-projects.
    #[serde(default)]
    pub(super) project_display: String,
}

/// A channel's human display name: its kind:39000 `name`, else the raw id.
fn display_name(store: &Store, id: &str) -> String {
    let channel = match store.get_channel(id) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(channel = %id, error = ?e, "who snapshot: get_channel failed resolving display name; falling back to raw id");
            None
        }
    };
    channel
        .map(|c| c.name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| id.to_string())
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
    // daemon's own host, so all rendering stays client-side. Local sessions are
    // on this machine by construction → never remote. A peer is remote ONLY when
    // its host differs from ours.
    let local_host = slugify_host(daemon_host);

    // Pubkeys this daemon signs as — used to drop our own relay echoes from the
    // peer set so a local session isn't double-counted as a remote one. A read
    // failure here would empty the self set and render our OWN sessions as remote
    // peers, so fail loud rather than mislabel.
    let my_pubkeys: std::collections::HashSet<String> = store
        .list_identity_pubkeys()
        .context("who snapshot: failed to load this daemon's identity pubkeys (self set)")?
        .into_iter()
        .collect();

    let mut rows = Vec::new();
    let mut other_agents: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();

    // ── local sessions on this machine ─────────────────────────────────────────
    // A read failure must error, not silently render "no local sessions".
    for s in store
        .list_alive_sessions()
        .context("who snapshot: failed to list live local sessions")?
    {
        let scope = s.channel_h.clone();
        if current_project.map(|p| p == scope).unwrap_or(true) {
            rows.push(local_row(store, &s, &local_host, now));
        } else if is_root_channel(store, &scope) {
            other_agents
                .entry(scope)
                .or_default()
                .insert(local_instance(store, &s).display_slug());
        }
    }

    // ── peers: relay_status across all channels, minus our own keys ────────────
    // Scan every channel even when a `current_project` is set: in-scope statuses
    // become rows, root channels out of scope feed the other-projects summary.
    let mut channels: Vec<String> = store
        .list_channels()
        .context("who snapshot: failed to list channels")?
        .into_iter()
        .map(|c| c.channel_h)
        .collect();
    if let Some(p) = current_project {
        if !channels.iter().any(|c| c == p) {
            channels.push(p.to_string());
        }
    }
    for ch in &channels {
        // A failed status read must not silently drop a channel's peers.
        let live = store
            .live_status_for_channel(ch, now)
            .with_context(|| format!("who snapshot: failed to read live status for {ch}"))?;
        for st in live {
            if my_pubkeys.contains(&st.pubkey) {
                continue;
            }
            let in_scope = current_project.map(|p| p == ch.as_str()).unwrap_or(true);
            if in_scope {
                rows.push(peer_row(store, &st, &local_host, now));
            } else if is_root_channel(store, ch) {
                let slug = peer_slug(store, &st);
                other_agents.entry(ch.clone()).or_default().insert(slug);
            }
        }
    }

    let other_projects = other_agents
        .into_iter()
        .map(|(project, agents)| {
            // not-found → no `about`; a genuine read error is logged loudly
            // rather than silently swallowed into the same None.
            let about = match store.get_channel(&project) {
                Ok(c) => c.map(|c| c.about).filter(|a| !a.is_empty()),
                Err(e) => {
                    tracing::error!(
                        channel = %project,
                        error = ?e,
                        "who snapshot: get_channel failed for other-project summary"
                    );
                    None
                }
            };
            // Show the project's human name; the raw id is only a fallback.
            let display = display_name(store, &project);
            let agents: Vec<String> = agents.into_iter().collect();
            OtherProjectSummary {
                project: display,
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

    // If the current scope is a session/task channel, surface its parent so the
    // renderer can label it as the channel (not the project). `parent` empty (or
    // unknown) ⇒ a top-level project, so `None`.
    let channel_parent = current_project.and_then(|p| {
        match store.channel_parent(p) {
            Ok(parent) => parent,
            Err(e) => {
                tracing::error!(channel = %p, error = ?e, "who snapshot: channel_parent lookup failed resolving current-scope parent");
                None
            }
        }
        .filter(|parent| !parent.is_empty())
    });

    let project_display = current_project
        .map(|p| display_name(store, p))
        .unwrap_or_else(|| "*".to_string());

    Ok(WhoSnapshot {
        project: current_project.unwrap_or("*").to_string(),
        now,
        rows,
        other_projects,
        spawnable,
        channel_parent,
        project_display,
    })
}

/// Top-level work-root for `scope`: walk `parent` links up to the first channel
/// whose parent is empty/unknown. Bounded to avoid cycles in malformed data.
fn work_root_for(store: &Store, scope: &str) -> String {
    let mut cur = scope.to_string();
    for _ in 0..16 {
        match store.channel_parent(&cur) {
            Ok(Some(parent)) if !parent.is_empty() => cur = parent,
            Ok(_) => break,
            Err(e) => {
                tracing::error!(
                    channel = %cur,
                    error = ?e,
                    "who snapshot: channel_parent lookup failed walking work-root"
                );
                break;
            }
        }
    }
    cur
}

fn is_root_channel(store: &Store, scope: &str) -> bool {
    match store.is_root_channel(scope) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(
                channel = %scope,
                error = ?e,
                "who snapshot: is_root_channel lookup failed; assuming root"
            );
            true
        }
    }
}

/// Build a local-session row. Title/activity/busy come from the agent's own
/// relay_status row when published, else the local pre-publish draft on the
/// session.
fn local_row(store: &Store, s: &crate::state::Session, local_host: &str, now: u64) -> WhoRow {
    let instance = local_instance(store, s);
    let live = store
        .get_status(&instance.pubkey, &s.channel_h)
        .ok()
        .flatten()
        .filter(|st| st.expiration == 0 || st.expiration >= now);
    let (title, activity, busy) = match live {
        Some(st) => (
            st.title,
            if st.busy { st.activity } else { String::new() },
            st.busy,
        ),
        None => (
            s.title.clone(),
            if s.working {
                s.activity.clone()
            } else {
                String::new()
            },
            s.working,
        ),
    };
    let fresh = now.saturating_sub(s.last_seen) <= crate::session::STATUS_TTL_SECS;
    WhoRow {
        source: WhoSource::Local,
        fresh,
        slug: instance.display_slug(),
        project: s.channel_h.clone(),
        status: title,
        activity,
        active: busy,
        host: local_host.to_string(),
        session_id: s.session_id.clone(),
        age_secs: Some(now.saturating_sub(s.last_seen)),
        rel_cwd: String::new(),
        remote: false,
        attachable: false,
        work_root: work_root_for(store, &s.channel_h),
        pubkey: instance.pubkey,
    }
}

fn local_instance(store: &Store, s: &crate::state::Session) -> crate::identity::AgentInstance {
    store
        .instance_identity_for_session(&s.session_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            crate::identity::AgentInstance::base(s.agent_slug.clone(), s.agent_pubkey.clone())
        })
}

/// Build a peer row from a relay-confirmed status. Host (and thus remoteness)
/// comes from the peer's kind:0 profile; an unknown host is treated as local.
fn peer_row(store: &Store, st: &crate::state::Status, local_host: &str, now: u64) -> WhoRow {
    let host = store
        .get_profile(&st.pubkey)
        .ok()
        .flatten()
        .map(|p| p.host)
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| local_host.to_string());
    let remote = slugify_host(&host) != local_host;
    WhoRow {
        source: WhoSource::Peer,
        fresh: true, // live_status_for_channel only returns unexpired rows
        slug: peer_slug(store, st),
        project: st.channel_h.clone(),
        status: st.title.clone(),
        activity: if st.busy {
            st.activity.clone()
        } else {
            String::new()
        },
        active: st.busy,
        host,
        session_id: String::new(),
        age_secs: Some(now.saturating_sub(st.last_seen)),
        rel_cwd: String::new(),
        remote,
        attachable: false,
        work_root: work_root_for(store, &st.channel_h),
        // Peer status is session-signed, so the status pubkey IS the address to
        // route to.
        pubkey: st.pubkey.clone(),
    }
}

fn peer_slug(store: &Store, st: &crate::state::Status) -> String {
    if !st.slug.is_empty() {
        return st.slug.clone();
    }
    store
        .resolve_slug_for_pubkey(&st.pubkey)
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::util::pubkey_short(&st.pubkey))
}

impl WhoSnapshot {
    /// Number of visible sessions (local + peer, including idle) in scope.
    pub fn session_count(&self) -> usize {
        self.rows.len()
    }
}
