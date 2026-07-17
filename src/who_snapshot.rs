use crate::state::Store;
use anyhow::Result;
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
    /// Agents mosaico has an identity for that can be spawned locally.
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
    pub(crate) state: crate::session_state::SessionState,
    pub(crate) slug: String,
    pub(crate) channel: String,
    /// Persistent session title (what the session is about); survives idle turns.
    pub(crate) status: String,
    /// Live "doing now" line published by a peer. Shown after the
    /// title while mid-turn; empty (and not rendered) when idle.
    #[serde(default)]
    pub(crate) activity: String,
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
    let aggregation = crate::who_aggregation::WhoAggregation::load(store, now)?;
    build_who_snapshot(&aggregation, current_root, now, daemon_host)
}

pub(crate) fn build_who_snapshot(
    aggregation: &crate::who_aggregation::WhoAggregation,
    current_root: Option<&str>,
    now: u64,
    daemon_host: &str,
) -> Result<WhoSnapshot> {
    // "Remote" is computed daemon-side by comparing each peer's backend label to
    // this daemon's; local sessions are on this daemon → never remote.
    let local_host = daemon_host.trim().to_string();

    // Pubkeys this daemon signs as — used to drop our own relay echoes from the
    // peer set. A read failure here would render our OWN sessions as remote peers,
    // so fail loud rather than mislabel.
    let my_pubkeys: HashSet<String> = aggregation.local_pubkeys.iter().cloned().collect();

    let mut rows = Vec::new();
    let mut other_agents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // ── local sessions on this machine (read failure must error, not go quiet) ──
    for s in &aggregation.local_sessions {
        let scope = s.channel_h.clone();
        if aggregation.channel(&scope).is_none() && aggregation.workspace_path(&scope).is_none() {
            continue;
        }
        if is_archived_channel(aggregation, &scope) {
            continue;
        }
        if current_root
            .map(|p| scope_contains_channel(aggregation, p, &scope))
            .transpose()?
            .unwrap_or(true)
        {
            rows.push(local_row(aggregation, s, &local_host, now)?);
        } else if is_root_channel(aggregation, &scope) {
            other_agents
                .entry(scope)
                .or_default()
                .insert(local_instance(aggregation, s).display_slug());
        }
    }
    dormant::push_claim_rows(
        aggregation,
        current_root,
        now,
        &local_host,
        &mut rows,
        &mut other_agents,
    )?;

    // ── peers: relay_status across all channels, minus our own keys ────────────
    // Scan every channel even when a `current_root` is set: in-scope statuses
    // become rows, root channels out of scope feed the other-workspaces summary.
    let mut channels: Vec<String> = aggregation
        .channels
        .iter()
        .filter(|c| !c.is_archived())
        .map(|c| c.channel_h.clone())
        .collect();
    if let Some(p) = current_root {
        if !is_archived_channel(aggregation, p) && !channels.iter().any(|c| c == p) {
            channels.push(p.to_string());
        }
    }
    for ch in &channels {
        // A failed status read must not silently drop a channel's peers.
        for st in aggregation.statuses_for(ch) {
            if my_pubkeys.contains(&st.pubkey) {
                continue;
            }
            let in_scope = current_root
                .map(|p| scope_contains_channel(aggregation, p, ch))
                .transpose()?
                .unwrap_or(true);
            if in_scope {
                rows.push(peer_row(aggregation, st, &local_host, now)?);
            } else if is_root_channel(aggregation, ch) {
                let slug = peer_slug(aggregation, st);
                other_agents.entry(ch.clone()).or_default().insert(slug);
            }
        }
    }

    let other_roots = other_agents
        .into_iter()
        .map(|(root, agents)| {
            // not-found → no `about`; a genuine read error is logged loudly
            // rather than silently swallowed into the same None.
            let about = aggregation
                .channel(&root)
                .map(|channel| channel.about.clone())
                .filter(|about| !about.is_empty());
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
    let roster_scope = current_root
        .map(|p| work_root_for(aggregation, p))
        .transpose()?;
    let mut seen_spawnable = BTreeSet::new();
    let mut spawnable: Vec<SpawnableRow> = aggregation
        .agents_for_root(roster_scope.as_deref())
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
    let channel_parent = current_root
        .and_then(|scope| aggregation.channel(scope))
        .map(|channel| channel.parent.clone())
        .filter(|parent| !parent.is_empty());

    let root_display = match (current_root, channel_parent.is_some()) {
        (Some(scope), true) => aggregation.channel_name(scope).to_string(),
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
