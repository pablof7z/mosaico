/// TUI data model and state management
use crate::util::SessionId;
use anyhow::Result;
use std::time::SystemTime;

pub(super) struct LiveRow {
    pub(super) slug: String,
    pub(super) host: String,
    pub(super) project: String,
    pub(super) session_id: String,       // full raw id for RPC calls
    pub(super) session_codename: String, // stable display codename (e.g. bravo4217)
    pub(super) status: String,
    pub(super) attachable: bool, // has a live tmux endpoint
}

pub(super) struct SpawnRow {
    pub(super) slug: String,
    pub(super) host: String,
    pub(super) command: String,
}

/// A pane to attach to once the event loop yields, plus the session to fall back
/// to if attaching fails because the pane is stale/gone. Attaching is best-effort:
/// the daemon's view of a live pane can be out of date, so a pane-not-found error
/// should never surface to the user — we just resume the session instead.
pub(super) struct PendingAttach {
    pub(super) pane: String,
    /// Session id to resume if attaching to `pane` fails. `None` for freshly
    /// spawned panes (nothing to resume — the spawn itself is the live session).
    pub(super) resume_sid: Option<String>,
}

pub(super) struct ResumeRow {
    pub(super) slug: String,
    pub(super) project: String,
    pub(super) session_id: String,       // full raw id for RPC calls
    pub(super) session_codename: String, // stable display codename (e.g. bravo4217)
    pub(super) title: String,
    pub(super) created_at: u64,
}

/// Tabs computed from live data: visible projects ordered by activity (live
/// first, then recently-active), plus hidden projects (>7 days inactive).
pub(super) struct ProjectTabs {
    /// Projects shown in the tab bar. Order: projects with live sessions first
    /// (alphabetically), then recently-active projects (alphabetically).
    pub(super) visible: Vec<String>,
    /// Projects with no activity in the past 7 days. Only reachable via search.
    pub(super) hidden: Vec<String>,
}

impl PartialEq for ProjectTabs {
    fn eq(&self, other: &Self) -> bool {
        self.visible == other.visible && self.hidden == other.hidden
    }
}

pub(super) enum TuiMode {
    Normal,
    Search { query: String, sel: usize },
}

pub(super) struct TuiData {
    pub(super) live: Vec<LiveRow>,
    pub(super) spawnable: Vec<SpawnRow>,
    pub(super) resumable: Vec<ResumeRow>,
}

const TWELVE_HOURS: u64 = 12 * 3600;

pub(super) fn compute_project_tabs(data: &TuiData) -> ProjectTabs {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Count live sessions per project.
    let mut live_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in &data.live {
        if !row.project.is_empty() {
            *live_count.entry(row.project.clone()).or_insert(0) += 1;
        }
    }
    let live_projects: std::collections::HashSet<String> = live_count.keys().cloned().collect();

    // Track latest created_at per project from resumable sessions.
    let mut last_active: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for row in &data.resumable {
        if !row.project.is_empty() {
            let e = last_active.entry(row.project.clone()).or_insert(0);
            *e = (*e).max(row.created_at);
        }
    }

    // Projects without live sessions: show if active within 12h, else hide.
    let mut visible_recent: Vec<String> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();

    for (proj, t) in &last_active {
        if live_projects.contains(proj) {
            continue;
        }
        if now.saturating_sub(*t) < TWELVE_HOURS {
            visible_recent.push(proj.clone());
        } else {
            hidden.push(proj.clone());
        }
    }
    visible_recent.sort();
    hidden.sort();

    // Sort live projects by session count descending, then alphabetically.
    let mut live_sorted: Vec<String> = live_projects.into_iter().collect();
    live_sorted.sort_by(|a, b| {
        let ca = live_count.get(a).copied().unwrap_or(0);
        let cb = live_count.get(b).copied().unwrap_or(0);
        cb.cmp(&ca).then(a.cmp(b))
    });

    let mut visible: Vec<String> = live_sorted;
    visible.extend(visible_recent);

    ProjectTabs { visible, hidden }
}

pub(super) fn tab_project(tabs: &[String], tab_idx: usize) -> Option<&str> {
    tabs.get(tab_idx).map(|s| s.as_str())
}

pub(super) fn filter_live<'a>(data: &'a TuiData, project_filter: &str) -> Vec<&'a LiveRow> {
    data.live
        .iter()
        .filter(|r| r.project == project_filter)
        .collect()
}

pub(super) fn filter_resumable<'a>(
    data: &'a TuiData,
    project_filter: &str,
    exited_hours: Option<u64>,
) -> Vec<&'a ResumeRow> {
    let hours = match exited_hours {
        None => return vec![],
        Some(h) => h,
    };
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cutoff = now.saturating_sub(hours * 3600);
    data.resumable
        .iter()
        .filter(|r| r.created_at >= cutoff && r.project == project_filter)
        .collect()
}

pub(super) fn row_project_for_tabs(row: &serde_json::Value) -> String {
    row["work_root"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| row["project"].as_str())
        .unwrap_or("")
        .to_string()
}

pub(super) fn update_tabs_after_refresh(data: &TuiData, pt: &mut ProjectTabs, tab_idx: &mut usize) {
    let mut new_pt = compute_project_tabs(data);
    // Preserve the currently-selected project tab even if it became "hidden"
    // (e.g., selected via fuzzy search but older than 12h).
    let current_proj = pt.visible.get(*tab_idx).cloned();
    if let Some(proj) = current_proj {
        if let Some(idx) = new_pt.visible.iter().position(|p| *p == proj) {
            *tab_idx = idx;
        } else if let Some(hi) = new_pt.hidden.iter().position(|p| *p == proj) {
            // Was hidden but user has it selected — keep it visible.
            let pinned = new_pt.hidden.remove(hi);
            new_pt.visible.push(pinned);
            *tab_idx = new_pt.visible.len() - 1;
        } else {
            *tab_idx = 0;
        }
    }
    *pt = new_pt;
}

/// Compute fuzzy matches for `query` across all projects (visible + hidden).
/// Case-insensitive substring match; visible projects listed first.
pub(super) fn fuzzy_matches(pt: &ProjectTabs, query: &str) -> Vec<String> {
    let q = query.to_lowercase();
    pt.visible
        .iter()
        .chain(pt.hidden.iter())
        .filter(|p| p.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

pub(super) fn fetch_tui_data() -> Result<TuiData> {
    let v = crate::daemon::blocking::call(
        "who",
        serde_json::json!({
            "project": null,
            "all_projects": true,
            "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        }),
    )?;

    let live = v["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|r| !r["remote"].as_bool().unwrap_or(false))
        .map(|r| {
            let raw_id = r["session_id"].as_str().unwrap_or("").to_string();
            let session_codename = SessionId::from(raw_id.as_str()).to_string();
            LiveRow {
                slug: r["slug"].as_str().unwrap_or("").to_string(),
                host: r["host"].as_str().unwrap_or("").to_string(),
                project: row_project_for_tabs(r),
                session_id: raw_id,
                session_codename,
                status: r["status"].as_str().unwrap_or("").to_string(),
                attachable: r["attachable"].as_bool().unwrap_or(false),
            }
        })
        .collect();

    let spawnable = v["spawnable"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|r| SpawnRow {
            slug: r["slug"].as_str().unwrap_or("").to_string(),
            host: r["host"].as_str().unwrap_or("").to_string(),
            command: r["command"].as_str().unwrap_or("").to_string(),
        })
        .collect();

    // Resumable (dead, but replayable) sessions come from a dedicated RPC.
    // Fail soft: an older daemon without it just yields an empty section.
    let resumable = crate::daemon::blocking::call("tmux_resumable", serde_json::json!({}))
        .ok()
        .and_then(|rv| rv["resumable"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let raw_id = r["session_id"].as_str().unwrap_or("").to_string();
            let session_codename = SessionId::from(raw_id.as_str()).to_string();
            ResumeRow {
                slug: r["slug"].as_str().unwrap_or("").to_string(),
                project: row_project_for_tabs(r),
                session_id: raw_id,
                session_codename,
                title: r["title"].as_str().unwrap_or("").to_string(),
                created_at: r["created_at"].as_u64().unwrap_or(0),
            }
        })
        .collect();

    Ok(TuiData {
        live,
        spawnable,
        resumable,
    })
}
