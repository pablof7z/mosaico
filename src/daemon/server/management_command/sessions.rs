//! Session listing for backend-addressed management commands.

use super::super::DaemonState;
use crate::state::{Status, Store};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

pub(super) fn list_sessions(state: &Arc<DaemonState>, scope_root: Option<&str>) -> Result<String> {
    let now = crate::util::now_secs();
    let summaries = state.with_store(|s| session_summaries_from_store(s, scope_root, now))?;
    if summaries.is_empty() {
        let scope = scope_root
            .map(|h| format!(" in {}", super::channel_label(state, h)))
            .unwrap_or_else(|| " in all channels".to_string());
        return Ok(format!("mgmt ok: no running sessions{scope}"));
    }

    let scope = scope_root
        .map(|h| format!(" under {}", super::channel_label(state, h)))
        .unwrap_or_else(|| " across all channels".to_string());
    let mut lines = vec![format!(
        "mgmt ok: {} running session(s){}",
        summaries.len(),
        scope
    )];
    for row in summaries {
        let state_word = if row.busy { "busy" } else { "idle" };
        let text = status_text(&row);
        lines.push(format!(
            "- @{} ({}) [{}] {}: {} (last active {})",
            row.agent,
            super::short(&row.session_id),
            row.channels.into_iter().collect::<Vec<_>>().join(", "),
            state_word,
            text,
            age(row.last_seen, now)
        ));
    }
    Ok(lines.join("\n"))
}

#[derive(Debug, Clone)]
struct SessionSummary {
    session_id: String,
    agent: String,
    channels: BTreeSet<String>,
    title: String,
    activity: String,
    busy: bool,
    last_seen: u64,
    updated_at: u64,
}

fn session_summaries_from_store(
    store: &Store,
    scope_root: Option<&str>,
    now: u64,
) -> Result<Vec<SessionSummary>> {
    let channels = store
        .list_channels()?
        .into_iter()
        .map(|c| (c.channel_h.clone(), c))
        .collect::<HashMap<_, _>>();
    let scope = scope_root.map(|root| channel_subtree(&channels, root));
    let mut rows: BTreeMap<(String, String), SessionSummary> = BTreeMap::new();
    for status in store.list_status_sessions(None, None)? {
        if status.expiration < now {
            continue;
        }
        if let Some(scope) = &scope {
            if !scope.contains(&status.channel_h) {
                continue;
            }
        }
        let label = channel_label_from_map(&channels, &status.channel_h);
        let profile = store.get_profile(&status.pubkey).ok().flatten();
        let agent = session_handle(&status, profile.as_ref());
        let key = (status.pubkey.clone(), status.session_id.clone());
        rows.entry(key)
            .and_modify(|row| {
                row.channels.insert(label.clone());
                if status.updated_at >= row.updated_at {
                    row.title = status.title.clone();
                    row.activity = status.activity.clone();
                    row.busy = status.busy;
                    row.updated_at = status.updated_at;
                }
                row.last_seen = row.last_seen.max(status.last_seen);
            })
            .or_insert_with(|| {
                let mut row = SessionSummary {
                    session_id: status.session_id.clone(),
                    agent,
                    channels: BTreeSet::new(),
                    title: status.title.clone(),
                    activity: status.activity.clone(),
                    busy: status.busy,
                    last_seen: status.last_seen,
                    updated_at: status.updated_at,
                };
                row.channels.insert(label);
                row
            });
    }
    let mut out = rows.into_values().collect::<Vec<_>>();
    out.sort_by(|a, b| {
        b.last_seen
            .cmp(&a.last_seen)
            .then_with(|| a.agent.cmp(&b.agent))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    Ok(out)
}

fn channel_subtree(
    channels: &HashMap<String, crate::state::Channel>,
    root: &str,
) -> BTreeSet<String> {
    let mut out = BTreeSet::from([root.to_string()]);
    for id in channels.keys() {
        if is_descendant(channels, id, root) {
            out.insert(id.clone());
        }
    }
    out
}

fn is_descendant(
    channels: &HashMap<String, crate::state::Channel>,
    channel_h: &str,
    root: &str,
) -> bool {
    let mut cur = channel_h;
    let mut guard = 0usize;
    while guard < 32 {
        guard += 1;
        if cur == root {
            return true;
        }
        let Some(channel) = channels.get(cur) else {
            return false;
        };
        if channel.parent.is_empty() {
            return false;
        }
        cur = &channel.parent;
    }
    false
}

fn session_handle(status: &Status, profile: Option<&crate::state::Profile>) -> String {
    let slug = if !status.slug.is_empty() {
        status.slug.as_str()
    } else if let Some(profile) = profile {
        profile.slug.as_str()
    } else {
        ""
    };
    if slug.is_empty() {
        return crate::util::pubkey_short(&status.pubkey);
    }
    if profile.is_some_and(|p| !p.agent_slug.is_empty()) {
        return slug.to_string();
    }
    let host = profile.map(|p| p.host.as_str()).unwrap_or_default();
    if host.is_empty() {
        slug.to_string()
    } else {
        crate::idref::agent_label(slug, host)
    }
}

fn channel_label_from_map(
    channels: &HashMap<String, crate::state::Channel>,
    channel_h: &str,
) -> String {
    channels
        .get(channel_h)
        .and_then(|c| c.human_name().map(str::to_string))
        .unwrap_or_else(|| channel_h.to_string())
}

fn status_text(row: &SessionSummary) -> String {
    let raw = if row.busy && !row.activity.trim().is_empty() {
        row.activity.trim()
    } else if !row.title.trim().is_empty() {
        row.title.trim()
    } else if !row.activity.trim().is_empty() {
        row.activity.trim()
    } else {
        "-"
    };
    truncate(raw, 96)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx == max_chars {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

fn age(ts: u64, now: u64) -> String {
    let secs = now.saturating_sub(ts);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 60 * 60 {
        format!("{}m ago", secs / 60)
    } else if secs < 24 * 60 * 60 {
        format!("{}h ago", secs / (60 * 60))
    } else {
        format!("{}d ago", secs / (24 * 60 * 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(pubkey: &str, session: &str, channel: &str, seen: u64) -> Status {
        Status {
            pubkey: pubkey.to_string(),
            session_id: session.to_string(),
            channel_h: channel.to_string(),
            slug: "coder".to_string(),
            title: "fixing parser".to_string(),
            activity: "running tests".to_string(),
            busy: true,
            last_seen: seen,
            updated_at: seen,
            expiration: seen + 90,
        }
    }

    #[test]
    fn scoped_session_list_includes_descendants_and_dedups_sessions() {
        let store = Store::open_memory().unwrap();
        store.upsert_channel("root", "proj", "", "", 1).unwrap();
        store
            .upsert_channel("child", "planning", "", "root", 2)
            .unwrap();
        store
            .upsert_channel("grandchild", "review", "", "child", 3)
            .unwrap();
        store.upsert_channel("other", "other", "", "", 4).unwrap();
        store
            .upsert_profile("pk1", "coder@laptop", "coder", "laptop", false, 1)
            .unwrap();
        store
            .upsert_status(&status("pk1", "s1", "root", 100))
            .unwrap();
        store
            .upsert_status(&status("pk1", "s1", "grandchild", 101))
            .unwrap();
        store
            .upsert_status(&status("pk2", "s2", "other", 102))
            .unwrap();

        let rows = session_summaries_from_store(&store, Some("root"), 110).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "s1");
        assert_eq!(
            rows[0].channels,
            BTreeSet::from(["proj".to_string(), "review".to_string()])
        );
    }
}
