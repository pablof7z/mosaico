use super::*;

/// `channels list`: render the subgroup tree under `channel` from LOCAL daemon
/// state (materialized kind:39000 metadata) — no relay round-trip. Returns the
/// rooms in depth-first order, each with a `depth` (the channel root is depth 0
/// and not included; its direct children are depth 1) so the CLI can indent.
pub(in crate::daemon::server) fn rpc_channels_list(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_list params")?;

    // Every channel the daemon has materialized.
    let rows = state.with_store(|s| s.list_channels())?;
    let rooms = channel_list_rooms(rows, &p.channel);
    Ok(serde_json::json!({ "channel": p.channel, "rooms": rooms }))
}

fn channel_list_rooms(rows: Vec<crate::state::Channel>, root: &str) -> Vec<serde_json::Value> {
    // parent id -> children (id, display name, about). Sorted for stable output.
    let mut children: std::collections::BTreeMap<String, Vec<(String, String, String)>> =
        std::collections::BTreeMap::new();
    for ch in rows {
        if ch.parent.is_empty() || ch.is_archived() {
            continue;
        }
        let display = if ch.name.is_empty() {
            ch.about.clone()
        } else {
            ch.name.clone()
        };
        children.entry(ch.parent.clone()).or_default().push((
            ch.channel_h.clone(),
            display,
            ch.about.clone(),
        ));
    }
    for v in children.values_mut() {
        v.sort();
    }

    preorder_rooms(&children, root)
}

/// Pre-order DFS flatten of the subgroup tree rooted at `root` into
/// `{child_h, name, about, depth}` JSON (root excluded, its children at depth 0).
fn preorder_rooms(
    children: &std::collections::BTreeMap<String, Vec<(String, String, String)>>,
    root: &str,
) -> Vec<serde_json::Value> {
    fn walk(
        children: &std::collections::BTreeMap<String, Vec<(String, String, String)>>,
        node: &str,
        depth: usize,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<serde_json::Value>,
    ) {
        if let Some(kids) = children.get(node) {
            for (child_id, name, about) in kids {
                if !seen.insert(child_id.clone()) {
                    continue;
                }
                out.push(serde_json::json!({
                    "child_h": child_id,
                    "name": name,
                    "about": about,
                    "depth": depth,
                }));
                walk(children, child_id, depth + 1, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(root.to_string());
    walk(children, root, 0, &mut seen, &mut out);
    out
}

#[cfg(test)]
mod tests;
