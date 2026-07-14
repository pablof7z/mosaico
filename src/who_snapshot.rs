use crate::state::{Store, StoreReader};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashSet};

mod dormant;
mod row_builders;
mod scope;

use row_builders::{local_instance, local_row, peer_row, peer_slug};
use scope::{is_archived_channel, is_root_channel, scope_contains_channel, work_root_for};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct OtherRootSummary {
    pub(crate) root: String,
    pub(crate) agent_count: usize,
    #[serde(default)]
    pub(crate) agents: Vec<String>,
    pub(crate) about: Option<String>,
}

// The daemon serializes a WhoSnapshot and the thin `who` client renders it with
// the EXACT renderers below — so output is byte-identical by construction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WhoSnapshot {
    pub(crate) root: String,
    pub(crate) now: u64,
    pub(crate) rows: Vec<WhoRow>,
    pub(crate) other_roots: Vec<OtherRootSummary>,
    /// Agents tenex-edge has an identity for that can be spawned locally.
    #[serde(default)]
    pub(crate) spawnable: Vec<SpawnableRow>,
    /// When the current scope is a per-session room, the work-root channel it is
    /// nested under. Lets the renderer label the room as the current *channel*
    /// (distinct from its *root channel*). `None` when the scope is a root channel.
    #[serde(default)]
    pub(crate) channel_parent: Option<String>,
    /// Human label for the scope: workspace id for roots, channel name for
    /// descendants, and `*` for all workspaces.
    #[serde(default)]
    pub(crate) root_display: String,
}

pub(super) fn display_name(store: StoreReader<'_>, id: &str) -> String {
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
pub(crate) struct SpawnableRow {
    pub(crate) host: String,
    pub(crate) slug: String,
    #[serde(default)]
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) byline: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct WhoRow {
    pub(crate) source: WhoSource,
    pub(crate) fresh: bool,
    pub(crate) slug: String,
    pub(crate) channel: String,
    /// Persistent session title (what the session is about); survives idle turns.
    pub(crate) status: String,
    /// Live "doing now" line, distilled alongside the title. Shown after the
    /// title while mid-turn; empty (and not rendered) when idle.
    #[serde(default)]
    pub(crate) activity: String,
    /// Whether the session is mid-turn. Drives the idle marker independently of
    /// the title, which is retained while idle.
    #[serde(default)]
    pub(crate) active: bool,
    /// A local Class A session that exited but still owns a soft route claim.
    #[serde(default)]
    pub(crate) dormant: bool,
    pub(crate) host: String,
    pub(crate) age_secs: Option<u64>,
    /// Channel-relative working dir (§8e). Empty or "." → rendered without a
    /// `[dir]` bracket; otherwise shown so worktrees render distinctly.
    #[serde(default)]
    pub(crate) rel_cwd: String,
    /// True only for a peer whose host differs from the daemon/viewer's host.
    /// Local sessions and same-machine peers are never remote (the §8e fix).
    #[serde(default)]
    pub(crate) remote: bool,
    /// True when this session has a live PTY endpoint registered.
    #[serde(default)]
    pub(crate) attachable: bool,
    /// Top-level work-root channel for UI grouping. `channel` remains the live
    /// routing scope (session room or task channel); this is the root-channel tab.
    #[serde(default)]
    pub(crate) work_root: String,
    /// Human display label for `work_root`.
    #[serde(default)]
    pub(crate) work_root_display: String,
    /// Hex pubkey others route to: per-session when derived, else durable agent.
    #[serde(default)]
    pub(crate) pubkey: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum WhoSource {
    Local,
    Peer,
}

pub(crate) fn load_who_snapshot(
    store: &Store,
    current_root: Option<&str>,
    now: u64,
    daemon_host: &str,
) -> Result<WhoSnapshot> {
    let store = store.reader();
    // "Remote" is computed daemon-side by comparing each peer's backend label to
    // this daemon's; local sessions are on this daemon → never remote.
    let local_host = daemon_host.trim().to_string();

    // Pubkeys this daemon signs as — used to drop our own relay echoes from the
    // peer set. A read failure here would render our OWN sessions as remote peers,
    // so fail loud rather than mislabel.
    let my_pubkeys: HashSet<String> = store
        .list_local_session_pubkeys()
        .context("who snapshot: failed to load this daemon's local session pubkeys")?
        .into_iter()
        .collect();

    let mut rows = Vec::new();
    let mut other_agents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // ── local sessions on this machine (read failure must error, not go quiet) ──
    for s in store
        .list_alive_sessions()
        .context("who snapshot: failed to list live local sessions")?
    {
        let scope = s.channel_h.clone();
        if is_archived_channel(store, &scope) {
            continue;
        }
        if current_root
            .map(|p| scope_contains_channel(store, p, &scope))
            .unwrap_or(true)
        {
            rows.push(local_row(store, &s, &local_host, now));
        } else if is_root_channel(store, &scope) {
            other_agents
                .entry(scope)
                .or_default()
                .insert(local_instance(store, &s).display_slug());
        }
    }
    dormant::push_claim_rows(
        store,
        current_root,
        now,
        &local_host,
        &mut rows,
        &mut other_agents,
    )
    .context("who snapshot: failed to read dormant session claims")?;

    // ── peers: relay_status across all channels, minus our own keys ────────────
    // Scan every channel even when a `current_root` is set: in-scope statuses
    // become rows, root channels out of scope feed the other-workspaces summary.
    let mut channels: Vec<String> = store
        .list_channels()
        .context("who snapshot: failed to list channels")?
        .into_iter()
        .filter(|c| !c.is_archived())
        .map(|c| c.channel_h)
        .collect();
    if let Some(p) = current_root {
        if !is_archived_channel(store, p) && !channels.iter().any(|c| c == p) {
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
            let in_scope = current_root
                .map(|p| scope_contains_channel(store, p, ch))
                .unwrap_or(true);
            if in_scope {
                rows.push(peer_row(store, &st, &local_host, now));
            } else if is_root_channel(store, ch) {
                let slug = peer_slug(store, &st);
                other_agents.entry(ch.clone()).or_default().insert(slug);
            }
        }
    }

    let other_roots = other_agents
        .into_iter()
        .map(|(root, agents)| {
            // not-found → no `about`; a genuine read error is logged loudly
            // rather than silently swallowed into the same None.
            let about = match store.get_channel(&root) {
                Ok(c) => c.map(|c| c.about).filter(|a| !a.is_empty()),
                Err(e) => {
                    tracing::error!(
                        channel = %root,
                        error = ?e,
                        "who snapshot: get_channel failed for other-root summary"
                    );
                    None
                }
            };
            let agents: Vec<String> = agents.into_iter().collect();
            OtherRootSummary {
                root,
                agent_count: agents.len(),
                agents,
                about,
            }
        })
        .collect();

    let local_spawnable = crate::session_host::spawnable_agents()
        .into_iter()
        .map(|(slug, command, byline)| (slug, (command, byline)))
        .collect::<BTreeMap<_, _>>();
    let roster_scope = current_root.map(|p| work_root_for(store, p));
    let mut seen_spawnable = BTreeSet::new();
    let mut spawnable: Vec<SpawnableRow> = match roster_scope.as_deref() {
        Some(root) => store.list_agent_roster_for_channel(root)?,
        None => store.list_agent_roster()?,
    }
    .into_iter()
    .map(|row| {
        let local = (row.host == local_host)
            .then(|| local_spawnable.get(&row.slug))
            .flatten();
        seen_spawnable.insert((row.host.clone(), row.slug.clone()));
        SpawnableRow {
            host: row.host,
            slug: row.slug,
            command: local
                .map(|(command, _)| command.clone())
                .unwrap_or_default(),
            byline: Some(row.use_criteria)
                .filter(|s| !s.trim().is_empty())
                .or_else(|| local.and_then(|(_, byline)| byline.clone())),
        }
    })
    .collect();
    for (slug, (command, byline)) in local_spawnable {
        if !seen_spawnable.insert((local_host.clone(), slug.clone())) {
            continue;
        }
        spawnable.push(SpawnableRow {
            host: local_host.clone(),
            slug,
            command,
            byline,
        });
    }
    spawnable.sort_by(|a, b| {
        a.host
            .cmp(&b.host)
            .then_with(|| a.slug.cmp(&b.slug))
            .then_with(|| a.command.cmp(&b.command))
    });

    // Session/task channel parent lets the renderer label channel vs root.
    let channel_parent = current_root.and_then(|p| {
        match store.channel_parent(p) {
            Ok(parent) => parent,
            Err(e) => {
                tracing::error!(channel = %p, error = ?e, "who snapshot: channel_parent lookup failed resolving current-scope parent");
                None
            }
        }
        .filter(|parent| !parent.is_empty())
    });

    let root_display = match (current_root, channel_parent.is_some()) {
        (Some(scope), true) => display_name(store, scope),
        (Some(scope), false) => scope.to_string(),
        (None, _) => "*".to_string(),
    };

    Ok(WhoSnapshot {
        root: current_root.unwrap_or("*").to_string(),
        now,
        rows,
        other_roots,
        spawnable,
        channel_parent,
        root_display,
    })
}
